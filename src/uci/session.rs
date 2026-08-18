use std::io::{self, BufRead, Write};

use crate::engine::{Engine, Position};

use super::command::{Command, PositionCommand, PositionSource, parse};

const ENGINE_NAME: &str = concat!("Jakgro ", env!("CARGO_PKG_VERSION"));
const ENGINE_AUTHOR: &str = "Jakgro contributors";

/// Runs a UCI session until `quit` or end of input.
pub fn run<R, W>(input: R, output: W) -> io::Result<()>
where
    R: BufRead,
    W: Write,
{
    Session::new(output).run(input)
}

struct Session<W> {
    engine: Engine,
    output: W,
    debug: bool,
}

impl<W> Session<W>
where
    W: Write,
{
    fn new(output: W) -> Self {
        Self {
            engine: Engine::new(),
            output,
            debug: false,
        }
    }

    fn run<R>(&mut self, mut input: R) -> io::Result<()>
    where
        R: BufRead,
    {
        let mut line = String::new();

        loop {
            line.clear();
            if input.read_line(&mut line)? == 0 {
                break;
            }

            match parse(&line) {
                Ok(command) => {
                    if !self.handle(command)? {
                        break;
                    }
                }
                Err(error) => self.debug_info(&format!("ignored command: {error}"))?,
            }
        }

        Ok(())
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
            Command::UciNewGame => self.engine.new_game(),
            Command::Position(command) => self.set_position(command)?,
            Command::Go(limits) => {
                let result = self.engine.search(&limits);
                match (result.best_move(), result.ponder()) {
                    (Some(best_move), Some(ponder)) => {
                        self.write_line(&format!("bestmove {best_move} ponder {ponder}"))?;
                    }
                    (Some(best_move), None) => {
                        self.write_line(&format!("bestmove {best_move}"))?;
                    }
                    (None, _) => self.write_line("bestmove 0000")?,
                }
            }
            Command::Stop | Command::PonderHit => {}
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

        self.engine.set_position(position);
        Ok(())
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
    use std::io::Cursor;

    use super::run;

    fn transcript(input: &str) -> String {
        let mut output = Vec::new();
        run(Cursor::new(input.as_bytes()), &mut output).unwrap();
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
    fn position_and_search_form_a_complete_transcript() {
        assert_eq!(
            transcript("position startpos moves e2e4 e7e5\ngo searchmoves g1f3 depth 1\nquit\n"),
            "bestmove g1f3\n"
        );
    }

    #[test]
    fn terminal_positions_return_the_null_move() {
        assert_eq!(
            transcript("position fen 7k/6Q1/6K1/8/8/8/8/8 b - - 0 1\ngo depth 1\nquit\n"),
            "bestmove 0000\n"
        );
    }

    #[test]
    fn rejected_positions_do_not_replace_engine_state() {
        assert_eq!(
            transcript(
                "position startpos moves e2e4\nposition startpos moves e2e5\ngo searchmoves e7e5\nquit\n"
            ),
            "bestmove e7e5 ponder a2a3\n"
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
}
