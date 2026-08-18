use std::io::{self, BufRead, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use crate::engine::{Engine, Position, SearchInfo, SearchLimits, SearchResult, SearchScore};

use super::command::{Command, PositionCommand, PositionSource, parse};
use super::event::Event;
use super::search_worker::{SearchEvent, SearchTask};

const ENGINE_NAME: &str = concat!("Jakgro ", env!("CARGO_PKG_VERSION"));
const ENGINE_AUTHOR: &str = "Jakgro contributors";

/// Runs a UCI session until `quit` or end of input.
///
/// Input is read on a helper thread so searches can report progress while waiting for commands.
/// Embedded callers should ensure their reader can eventually unblock or reach end of input.
pub fn run<R, W>(input: R, output: W) -> io::Result<()>
where
    R: BufRead + Send + 'static,
    W: Write,
{
    let (sender, receiver) = mpsc::channel();
    let input_sender = sender.clone();
    let panic_sender = input_sender.clone();
    let _reader = thread::spawn(move || {
        if catch_unwind(AssertUnwindSafe(|| pump_input(input, input_sender))).is_err() {
            let _ = panic_sender.send(Event::InputPanicked);
        }
    });
    Session::new(output, sender).run(receiver)
}

fn pump_input<R>(mut input: R, sender: Sender<Event>)
where
    R: BufRead,
{
    let mut line = String::new();
    loop {
        line.clear();
        match input.read_line(&mut line) {
            Ok(0) => {
                let _ = sender.send(Event::InputClosed);
                break;
            }
            Ok(_) => {
                if sender.send(Event::Input(parse(&line))).is_err() {
                    break;
                }
            }
            Err(error) => {
                let _ = sender.send(Event::InputFailed(error));
                break;
            }
        }
    }
}

struct Session<W> {
    engine: Engine,
    output: W,
    debug: bool,
    sender: Sender<Event>,
    active: Option<SearchTask>,
    next_generation: u64,
}

impl<W> Session<W>
where
    W: Write,
{
    fn new(output: W, sender: Sender<Event>) -> Self {
        Self {
            engine: Engine::new(),
            output,
            debug: false,
            sender,
            active: None,
            next_generation: 1,
        }
    }

    fn run(mut self, receiver: Receiver<Event>) -> io::Result<()> {
        let result = self.run_loop(&receiver);
        self.cancel_active();
        result
    }

    fn run_loop(&mut self, receiver: &Receiver<Event>) -> io::Result<()> {
        loop {
            let event = receiver.recv().map_err(|_| {
                io::Error::new(io::ErrorKind::BrokenPipe, "UCI event channel closed")
            })?;
            match event {
                Event::Input(Ok(command)) => {
                    if !self.handle(command)? {
                        return Ok(());
                    }
                }
                Event::Input(Err(error)) => {
                    self.debug_info(&format!("ignored command: {error}"))?;
                }
                Event::InputClosed => return Ok(()),
                Event::InputFailed(error) => return Err(error),
                Event::InputPanicked => {
                    return Err(io::Error::other("UCI input reader panicked"));
                }
                Event::Search(event) => self.handle_search_event(event)?,
            }
        }
    }

    fn handle(&mut self, command: Command) -> io::Result<bool> {
        match command {
            Command::Empty => {}
            Command::Uci => self.identify()?,
            Command::Debug(enabled) => self.debug = enabled,
            Command::IsReady => self.write_line("readyok")?,
            Command::SetOption { name, value: _ } => {
                self.debug_info(&format!("unsupported option: {name}"))?;
            }
            Command::UciNewGame => {
                self.cancel_active();
                self.engine.new_game();
            }
            Command::Position(command) => self.set_position(command)?,
            Command::Go(limits) => self.start_search(limits),
            Command::Stop => self.stop_search()?,
            Command::PonderHit => self.ponder_hit()?,
            Command::Quit => return Ok(false),
            Command::Unknown(name) => {
                self.debug_info(&format!("ignored unknown command: {name}"))?;
            }
        }

        Ok(true)
    }

    fn identify(&mut self) -> io::Result<()> {
        writeln!(self.output, "id name {ENGINE_NAME}")?;
        writeln!(self.output, "id author {ENGINE_AUTHOR}")?;
        writeln!(self.output, "uciok")?;
        self.output.flush()
    }

    fn set_position(&mut self, command: PositionCommand) -> io::Result<()> {
        let mut position = match command.source {
            PositionSource::StartPosition => Position::default(),
            PositionSource::Fen(fen) => match Position::from_fen(&fen) {
                Ok(position) => position,
                Err(error) => {
                    return self.debug_info(&format!("position rejected: {error}"));
                }
            },
        };

        if let Err(error) = position.apply_uci_moves(&command.moves) {
            return self.debug_info(&format!("position rejected: {error}"));
        }

        self.cancel_active();
        self.engine.set_position(position);
        Ok(())
    }

    fn start_search(&mut self, limits: SearchLimits) {
        self.cancel_active();
        let generation = self.next_generation;
        self.next_generation = self.next_generation.wrapping_add(1).max(1);
        self.active = Some(SearchTask::spawn(
            self.engine.clone(),
            limits,
            generation,
            self.sender.clone(),
        ));
    }

    fn stop_search(&mut self) -> io::Result<()> {
        let pending = self.active.as_mut().and_then(SearchTask::stop_and_release);
        if let Some(result) = pending {
            self.write_search_result(result)?;
            self.active = None;
        }
        Ok(())
    }

    fn ponder_hit(&mut self) -> io::Result<()> {
        let pending = self.active.as_mut().and_then(SearchTask::ponder_hit);
        if let Some(result) = pending {
            self.write_search_result(result)?;
            self.active = None;
        }
        Ok(())
    }

    fn handle_search_event(&mut self, event: SearchEvent) -> io::Result<()> {
        match event {
            SearchEvent::Info { generation, info } => {
                if self
                    .active
                    .as_ref()
                    .is_some_and(|task| task.generation() == generation)
                {
                    self.write_search_info(&info)?;
                }
            }
            SearchEvent::Finished { generation, result } => {
                if self
                    .active
                    .as_ref()
                    .is_none_or(|task| task.generation() != generation)
                {
                    return Ok(());
                }
                let released = self.active.as_mut().and_then(|task| task.complete(result));
                if let Some(result) = released {
                    self.write_search_result(result)?;
                    self.active = None;
                }
            }
            SearchEvent::Failed { generation } => {
                if self
                    .active
                    .as_ref()
                    .is_none_or(|task| task.generation() != generation)
                {
                    return Ok(());
                }
                let released = self.active.as_mut().and_then(SearchTask::fail);
                self.write_line("info string search worker failed")?;
                if let Some(result) = released {
                    self.write_search_result(result)?;
                    self.active = None;
                }
            }
        }
        Ok(())
    }

    fn cancel_active(&mut self) {
        if let Some(mut task) = self.active.take() {
            task.cancel();
        }
    }

    fn write_search_info(&mut self, info: &SearchInfo) -> io::Result<()> {
        let score = match info.score() {
            SearchScore::Centipawns(score) => format!("cp {score}"),
            SearchScore::Mate(moves) => format!("mate {moves}"),
        };
        let elapsed = info.elapsed().as_millis().min(u128::from(u64::MAX));
        let mut line = format!(
            "info depth {} score {score} nodes {} time {elapsed} nps {}",
            info.depth(),
            info.nodes(),
            info.nodes_per_second()
        );
        if !info.pv().is_empty() {
            line.push_str(" pv ");
            line.push_str(&info.pv().join(" "));
        }
        self.write_line(&line)
    }

    fn write_search_result(&mut self, result: SearchResult) -> io::Result<()> {
        match (result.best_move(), result.ponder()) {
            (Some(best_move), Some(ponder)) => {
                self.write_line(&format!("bestmove {best_move} ponder {ponder}"))
            }
            (Some(best_move), None) => self.write_line(&format!("bestmove {best_move}")),
            (None, _) => self.write_line("bestmove 0000"),
        }
    }

    fn debug_info(&mut self, message: &str) -> io::Result<()> {
        if self.debug {
            self.write_line(&format!("info string {message}"))?;
        }
        Ok(())
    }

    fn write_line(&mut self, line: &str) -> io::Result<()> {
        writeln!(self.output, "{line}")?;
        self.output.flush()
    }
}

#[cfg(test)]
mod tests {
    use std::io::{self, BufRead, Cursor, Read};

    use super::run;

    fn transcript(input: &str) -> String {
        let mut output = Vec::new();
        run(Cursor::new(input.as_bytes().to_vec()), &mut output).unwrap();
        String::from_utf8(output).unwrap()
    }

    #[test]
    fn handshake_and_readiness_have_exact_output() {
        assert_eq!(
            transcript("uci\nisready\nquit\n"),
            concat!(
                "id name Jakgro ",
                env!("CARGO_PKG_VERSION"),
                "\nid author Jakgro contributors\nuciok\nreadyok\n"
            )
        );
    }

    #[test]
    fn malformed_and_unknown_commands_are_silent_by_default() {
        assert_eq!(
            transcript("go depth nope\nunknown command\nisready extra\nisready\nquit\n"),
            "readyok\n"
        );
    }

    #[test]
    fn debug_mode_reports_ignored_commands() {
        let output = transcript("debug on\ngo depth nope\nnonsense\nquit\n");

        assert!(output.contains("info string ignored command: invalid depth value: nope\n"));
        assert!(output.contains("info string ignored unknown command: nonsense\n"));
    }

    #[test]
    fn end_of_input_ends_the_session_cleanly() {
        assert_eq!(transcript(""), "");
    }

    struct PanickingReader;

    impl Read for PanickingReader {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            panic!("input failure")
        }
    }

    impl BufRead for PanickingReader {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            panic!("input failure")
        }

        fn consume(&mut self, _amount: usize) {}
    }

    #[test]
    fn input_thread_panics_are_reported() {
        let mut output = Vec::new();

        let error = run(PanickingReader, &mut output).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Other);
    }
}
