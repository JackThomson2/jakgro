use std::fmt::{self, Display, Formatter};
use std::time::Duration;

use crate::engine::SearchLimits;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Command {
    Empty,
    Uci,
    Debug(bool),
    IsReady,
    SetOption { name: String, value: Option<String> },
    UciNewGame,
    Position(PositionCommand),
    Go(SearchLimits),
    Stop,
    PonderHit,
    Quit,
    Unknown(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PositionCommand {
    pub(super) source: PositionSource,
    pub(super) moves: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum PositionSource {
    StartPosition,
    Fen(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CommandError {
    message: String,
}

impl CommandError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Display for CommandError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

pub(super) fn parse(line: &str) -> Result<Command, CommandError> {
    let mut words = line.split_whitespace();
    let Some(command) = words.next() else {
        return Ok(Command::Empty);
    };
    let arguments = words.collect::<Vec<_>>();

    match command {
        "uci" => no_arguments(command, &arguments, Command::Uci),
        "debug" => parse_debug(&arguments),
        "isready" => no_arguments(command, &arguments, Command::IsReady),
        "setoption" => parse_set_option(&arguments),
        "ucinewgame" => no_arguments(command, &arguments, Command::UciNewGame),
        "position" => parse_position(&arguments),
        "go" => parse_go(&arguments),
        "stop" => no_arguments(command, &arguments, Command::Stop),
        "ponderhit" => no_arguments(command, &arguments, Command::PonderHit),
        "quit" => no_arguments(command, &arguments, Command::Quit),
        other => Ok(Command::Unknown(other.to_owned())),
    }
}

fn no_arguments(
    command_name: &str,
    arguments: &[&str],
    command: Command,
) -> Result<Command, CommandError> {
    if let Some(argument) = arguments.first() {
        Err(CommandError::new(format!(
            "unexpected argument for {command_name}: {argument}"
        )))
    } else {
        Ok(command)
    }
}

fn parse_debug(arguments: &[&str]) -> Result<Command, CommandError> {
    match arguments {
        ["on"] => Ok(Command::Debug(true)),
        ["off"] => Ok(Command::Debug(false)),
        [] => Err(CommandError::new("debug requires on or off")),
        _ => Err(CommandError::new("debug accepts exactly on or off")),
    }
}

fn parse_set_option(arguments: &[&str]) -> Result<Command, CommandError> {
    if arguments.first() != Some(&"name") {
        return Err(CommandError::new("setoption requires name"));
    }

    let value_index = arguments.iter().position(|argument| *argument == "value");
    let name_end = value_index.unwrap_or(arguments.len());
    let name = arguments[1..name_end].join(" ");
    if name.is_empty() {
        return Err(CommandError::new("setoption requires a non-empty name"));
    }

    let value = value_index.map(|index| arguments[index + 1..].join(" "));
    Ok(Command::SetOption { name, value })
}

fn parse_position(arguments: &[&str]) -> Result<Command, CommandError> {
    let Some(source) = arguments.first() else {
        return Err(CommandError::new(
            "position requires startpos or a six-field FEN",
        ));
    };

    let (source, remaining) = match *source {
        "startpos" => (PositionSource::StartPosition, &arguments[1..]),
        "fen" => {
            if arguments.len() < 7 {
                return Err(CommandError::new("position fen requires six FEN fields"));
            }
            (
                PositionSource::Fen(arguments[1..7].join(" ")),
                &arguments[7..],
            )
        }
        other => {
            return Err(CommandError::new(format!(
                "unknown position source: {other}"
            )));
        }
    };

    let moves = match remaining {
        [] => Vec::new(),
        ["moves", moves @ ..] => moves
            .iter()
            .map(|move_text| (*move_text).to_owned())
            .collect(),
        [unexpected, ..] => {
            return Err(CommandError::new(format!(
                "unexpected position argument: {unexpected}"
            )));
        }
    };

    Ok(Command::Position(PositionCommand { source, moves }))
}

fn parse_go(arguments: &[&str]) -> Result<Command, CommandError> {
    let mut limits = SearchLimits::default();
    let mut index = 0;

    while index < arguments.len() {
        match arguments[index] {
            "searchmoves" => {
                index += 1;
                let start = index;
                while index < arguments.len() && !is_go_keyword(arguments[index]) {
                    limits.search_moves.push(arguments[index].to_owned());
                    index += 1;
                }
                if index == start {
                    return Err(CommandError::new("searchmoves requires at least one move"));
                }
            }
            "ponder" => {
                limits.ponder = true;
                index += 1;
            }
            "wtime" => {
                limits.white_time = Some(parse_milliseconds(arguments, &mut index, "wtime")?);
            }
            "btime" => {
                limits.black_time = Some(parse_milliseconds(arguments, &mut index, "btime")?);
            }
            "winc" => {
                limits.white_increment = Some(parse_milliseconds(arguments, &mut index, "winc")?);
            }
            "binc" => {
                limits.black_increment = Some(parse_milliseconds(arguments, &mut index, "binc")?);
            }
            "movestogo" => {
                limits.moves_to_go = Some(parse_u32(arguments, &mut index, "movestogo")?);
            }
            "depth" => {
                limits.depth = Some(parse_u32(arguments, &mut index, "depth")?);
            }
            "nodes" => {
                limits.nodes = Some(parse_u64(arguments, &mut index, "nodes")?);
            }
            "mate" => {
                limits.mate = Some(parse_u32(arguments, &mut index, "mate")?);
            }
            "movetime" => {
                limits.move_time = Some(parse_milliseconds(arguments, &mut index, "movetime")?);
            }
            "infinite" => {
                limits.infinite = true;
                index += 1;
            }
            unknown => {
                return Err(CommandError::new(format!("unknown go argument: {unknown}")));
            }
        }
    }

    Ok(Command::Go(limits))
}

fn is_go_keyword(argument: &str) -> bool {
    matches!(
        argument,
        "searchmoves"
            | "ponder"
            | "wtime"
            | "btime"
            | "winc"
            | "binc"
            | "movestogo"
            | "depth"
            | "nodes"
            | "mate"
            | "movetime"
            | "infinite"
    )
}

fn parse_milliseconds(
    arguments: &[&str],
    index: &mut usize,
    name: &str,
) -> Result<Duration, CommandError> {
    parse_u64(arguments, index, name).map(Duration::from_millis)
}

fn parse_u32(arguments: &[&str], index: &mut usize, name: &str) -> Result<u32, CommandError> {
    let value = take_value(arguments, index, name)?;
    value
        .parse()
        .map_err(|_| CommandError::new(format!("invalid {name} value: {value}")))
}

fn parse_u64(arguments: &[&str], index: &mut usize, name: &str) -> Result<u64, CommandError> {
    let value = take_value(arguments, index, name)?;
    value
        .parse()
        .map_err(|_| CommandError::new(format!("invalid {name} value: {value}")))
}

fn take_value<'a>(
    arguments: &'a [&str],
    index: &mut usize,
    name: &str,
) -> Result<&'a str, CommandError> {
    let value = arguments
        .get(*index + 1)
        .copied()
        .ok_or_else(|| CommandError::new(format!("{name} requires a value")))?;
    *index += 2;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{Command, PositionCommand, PositionSource, parse};
    use crate::engine::SearchLimits;

    #[test]
    fn parses_blank_input() {
        assert_eq!(parse("  \t ").unwrap(), Command::Empty);
    }

    #[test]
    fn parses_start_position_with_moves() {
        assert_eq!(
            parse("position startpos moves e2e4 e7e5").unwrap(),
            Command::Position(PositionCommand {
                source: PositionSource::StartPosition,
                moves: vec!["e2e4".to_owned(), "e7e5".to_owned()],
            })
        );
    }

    #[test]
    fn parses_fen_position_with_moves() {
        assert_eq!(
            parse("position fen 4k3/8/8/8/8/8/8/4K3 w - - 0 1 moves e1e2").unwrap(),
            Command::Position(PositionCommand {
                source: PositionSource::Fen("4k3/8/8/8/8/8/8/4K3 w - - 0 1".to_owned()),
                moves: vec!["e1e2".to_owned()],
            })
        );
    }

    #[test]
    fn parses_all_go_limits() {
        assert_eq!(
            parse(
                "go searchmoves e2e4 d2d4 ponder wtime 1000 btime 2000 winc 10 binc 20 movestogo 30 depth 8 nodes 900 mate 3 movetime 50 infinite"
            )
            .unwrap(),
            Command::Go(SearchLimits {
                search_moves: vec!["e2e4".to_owned(), "d2d4".to_owned()],
                ponder: true,
                white_time: Some(Duration::from_millis(1000)),
                black_time: Some(Duration::from_millis(2000)),
                white_increment: Some(Duration::from_millis(10)),
                black_increment: Some(Duration::from_millis(20)),
                moves_to_go: Some(30),
                depth: Some(8),
                nodes: Some(900),
                mate: Some(3),
                move_time: Some(Duration::from_millis(50)),
                infinite: true,
            })
        );
    }

    #[test]
    fn parses_option_names_and_values_with_spaces() {
        assert_eq!(
            parse("setoption name Aggressive Style value maximum pressure").unwrap(),
            Command::SetOption {
                name: "Aggressive Style".to_owned(),
                value: Some("maximum pressure".to_owned()),
            }
        );
    }

    #[test]
    fn rejects_invalid_numeric_limits() {
        assert_eq!(
            parse("go depth nope").unwrap_err().to_string(),
            "invalid depth value: nope"
        );
    }

    #[test]
    fn rejects_incomplete_fen() {
        assert_eq!(
            parse("position fen 8/8/8/8/8/8/8/8 w - -")
                .unwrap_err()
                .to_string(),
            "position fen requires six FEN fields"
        );
    }
}
