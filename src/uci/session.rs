use std::io::{self, BufRead, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use crate::engine::{
    DEFAULT_AGGRESSION, DEFAULT_HASH_MIB, DEFAULT_MOVE_OVERHEAD_MS, DEFAULT_THREADS, Engine,
    MAX_AGGRESSION, MAX_HASH_MIB, MAX_MOVE_OVERHEAD_MS, MAX_THREADS, MIN_AGGRESSION, MIN_HASH_MIB,
    MIN_MOVE_OVERHEAD_MS, MIN_THREADS, Position, SearchInfo, SearchLimits, SearchResult,
    SearchScore,
};

use super::command::{Command, PositionCommand, PositionSource, parse};
use super::event::Event;
use super::search_worker::{SearchEvent, SearchTask};

const ENGINE_NAME: &str = concat!("Jakgro ", env!("CARGO_PKG_VERSION"));
const ENGINE_AUTHOR: &str = "Jakgro contributors";
fn parse_clamped_spin(value: &str, minimum: u64, maximum: u64) -> Option<u64> {
    let value = value.trim();
    let (negative, digits) = if let Some(digits) = value.strip_prefix('-') {
        (true, digits)
    } else {
        (false, value.strip_prefix('+').unwrap_or(value))
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if negative {
        return Some(minimum);
    }
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        return Some(minimum);
    }
    let upper = maximum.to_string();
    if digits.len() > upper.len() || (digits.len() == upper.len() && digits > upper.as_str()) {
        return Some(maximum);
    }
    digits
        .parse::<u64>()
        .ok()
        .map(|value| value.clamp(minimum, maximum))
}

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
            Command::SetOption { name, value } => self.set_option(&name, value.as_deref())?,
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
        writeln!(
            self.output,
            "option name Hash type spin default {DEFAULT_HASH_MIB} min {MIN_HASH_MIB} max {MAX_HASH_MIB}",
        )?;
        writeln!(
            self.output,
            "option name Threads type spin default {DEFAULT_THREADS} min {MIN_THREADS} max {MAX_THREADS}",
        )?;
        writeln!(
            self.output,
            "option name Aggression type spin default {DEFAULT_AGGRESSION} min {MIN_AGGRESSION} max {MAX_AGGRESSION}",
        )?;
        writeln!(
            self.output,
            "option name Move Overhead type spin default {DEFAULT_MOVE_OVERHEAD_MS} min {MIN_MOVE_OVERHEAD_MS} max {MAX_MOVE_OVERHEAD_MS}",
        )?;
        writeln!(self.output, "option name Clear Hash type button")?;
        writeln!(self.output, "uciok")?;
        self.output.flush()
    }

    fn set_option(&mut self, name: &str, value: Option<&str>) -> io::Result<()> {
        if name.eq_ignore_ascii_case("Hash") {
            let Some(value) = value.filter(|value| !value.is_empty()) else {
                return self.debug_info("Hash requires a size in MiB");
            };
            let Ok(size_mib) = value.parse::<usize>() else {
                return self.debug_info(&format!("invalid Hash value: {value}"));
            };

            self.cancel_active();
            if let Err(error) = self.engine.set_hash_size_mib(size_mib) {
                return self.debug_info(&format!("Hash rejected: {error}"));
            }
            return Ok(());
        }

        if name.eq_ignore_ascii_case("Threads") {
            let Some(value) = value.filter(|value| !value.is_empty()) else {
                return self.debug_info("Threads requires a count");
            };
            let Some(threads) = parse_clamped_spin(value, MIN_THREADS as u64, MAX_THREADS as u64)
            else {
                return self.debug_info(&format!("invalid Threads value: {value}"));
            };

            self.cancel_active();
            self.engine.set_threads(threads as usize);
            return Ok(());
        }

        if name.eq_ignore_ascii_case("Clear Hash") {
            if value.is_some_and(|value| !value.is_empty()) {
                return self.debug_info("Clear Hash does not accept a value");
            }

            self.cancel_active();
            self.engine.clear_hash();
            return Ok(());
        }

        if name.eq_ignore_ascii_case("Aggression") {
            let Some(value) = value.filter(|value| !value.is_empty()) else {
                return self.debug_info("Aggression requires a value from 0 to 100");
            };
            let Some(aggression) =
                parse_clamped_spin(value, u64::from(MIN_AGGRESSION), u64::from(MAX_AGGRESSION))
            else {
                return self.debug_info(&format!("invalid Aggression value: {value}"));
            };
            let aggression = aggression as u8;
            self.engine.set_aggression(aggression);
            return Ok(());
        }
        if name.eq_ignore_ascii_case("Move Overhead") {
            let Some(value) = value.filter(|value| !value.is_empty()) else {
                return self.debug_info("Move Overhead requires a value in milliseconds");
            };
            let Some(milliseconds) =
                parse_clamped_spin(value, MIN_MOVE_OVERHEAD_MS, MAX_MOVE_OVERHEAD_MS)
            else {
                return self.debug_info(&format!("invalid Move Overhead value: {value}"));
            };
            self.engine
                .set_move_overhead(Duration::from_millis(milliseconds));
            return Ok(());
        }

        self.debug_info(&format!("unsupported option: {name}"))
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
                let released = self.active.as_mut().and_then(|task| task.complete(*result));
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

    use super::{parse_clamped_spin, run};

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
                "\nid author Jakgro contributors\noption name Hash type spin default 16 min 1 max 1024\noption name Threads type spin default 1 min 1 max 128\noption name Aggression type spin default 75 min 0 max 100\noption name Move Overhead type spin default 10 min 0 max 5000\noption name Clear Hash type button\nuciok\nreadyok\n"
            )
        );
    }
    #[test]
    fn spin_values_clamp_without_fixed_width_integer_overflow() {
        assert_eq!(parse_clamped_spin("+00037", 0, 100), Some(37));
        assert_eq!(
            parse_clamped_spin("999999999999999999999999999999", 0, 100),
            Some(100),
        );
        assert_eq!(
            parse_clamped_spin("-999999999999999999999999999999", 0, 100),
            Some(0),
        );
        assert_eq!(parse_clamped_spin("nope", 0, 100), None);
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
    fn hash_options_resize_and_clear_without_protocol_noise() {
        assert_eq!(
            transcript("setoption name Hash value 2\nsetoption name Clear Hash\nisready\nquit\n",),
            "readyok\n",
        );
    }
    #[test]
    fn persistent_spin_options_accept_and_clamp_values_without_protocol_noise() {
        assert_eq!(
            transcript(
                "setoption name Aggression value 37\nsetoption name Aggression value -1\nsetoption name Aggression value 999999\nsetoption name Move Overhead value 250\nsetoption name Move Overhead value -1\nsetoption name Move Overhead value 999999\nsetoption name Threads value 4\nsetoption name Threads value 0\nsetoption name Threads value 999999\nucinewgame\nisready\nquit\n",
            ),
            "readyok\n",
        );
    }

    #[test]
    fn debug_mode_reports_invalid_options() {
        let output = transcript(
            "debug on\nsetoption name Hash value 0\nsetoption name Hash value nope\nsetoption name Clear Hash value nope\nsetoption name Aggression value nope\nsetoption name Move Overhead value nope\nquit\n",
        );

        assert!(
            output.contains("info string Hash rejected: hash size 0 MiB is outside 1..=1024 MiB\n")
        );
        assert!(output.contains("info string invalid Hash value: nope\n"));
        assert!(output.contains("info string Clear Hash does not accept a value\n"));
        assert!(output.contains("info string invalid Aggression value: nope\n"));
        assert!(output.contains("info string invalid Move Overhead value: nope\n"));
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
