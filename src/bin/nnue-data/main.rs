//! Strict, streaming feature preparation, integer inference and CPU training
//! for offline NNUE tools.

mod prepare;
mod sha256;
mod train;

use std::io::{self, BufRead, BufReader, Read, Write};
use std::process::ExitCode;
use std::sync::LazyLock;

use cozy_chess::{BitBoard, Board, Color, GameStatus, Piece};
use jakgro::engine::nnue::{FEATURE_SET, INPUT_FEATURES, Network};

const MAX_LINE_BYTES: u64 = 4096;
/// Dataset schema line: the feature contract the rows were extracted for.
///
/// Rows hold feature indices, which depend on the feature count and the
/// feature-set version and on nothing else about the network, so a change of
/// hidden width or activation never invalidates a corpus.
static SCHEMA: LazyLock<String> =
    LazyLock::new(|| format!("# jakgro-nnue-data-v2\t{INPUT_FEATURES}\t{FEATURE_SET}"));
/// Column names of a prepared row.
const COLUMNS: &str = "fen\tkey\tstm\toutcome\tteacher_cp\twhite\tblack";
/// The two header lines written before the rows of a prepared split.
static HEADER: LazyLock<String> = LazyLock::new(|| format!("{}\n{COLUMNS}\n", *SCHEMA));

/// Whether a prepared split's schema line describes this feature contract.
///
/// Version 1 lines also recorded the hidden width, activation ceiling and
/// output scale of the helper that wrote them (`v1`, features, hidden,
/// activation, output scale). Rows never depended on those three fields, so
/// a version 1 dataset with this feature count stays loadable and only the
/// feature count is checked.
fn schema_accepted(line: &str) -> bool {
    if line == SCHEMA.as_str() {
        return true;
    }
    let mut fields = line.split('\t');
    fields.next() == Some("# jakgro-nnue-data-v1")
        && fields.next() == Some(INPUT_FEATURES.to_string().as_str())
        && fields.count() == 3
}

fn main() -> ExitCode {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    let result = match arguments.as_slice() {
        [command, rest @ ..] if command == "prepare" => prepare::Options::parse(rest)
            .and_then(|options| prepare::prepare(&options))
            .map(|summary| {
                println!("{summary}");
                "dataset prepared".to_owned()
            }),
        [command, model, input] if command == "score" => Network::load(model)
            .map_err(|error| error.to_string())
            .and_then(|network| {
                open(input).and_then(|reader| score(&network, reader, io::stdout().lock()))
            })
            .map(|count| format!("{count} validated positions")),
        [command, rest @ ..] if command == "train" => {
            train::Options::parse(rest).and_then(|(options, data, output)| {
                train::train(&options, &data, &output).map(|summary| {
                    println!("{summary}");
                    format!("network written to {}", output.display())
                })
            })
        }
        _ => Err(
            "usage: nnue-data prepare --training TXT --development TXT --output-dir DIR \
                 [--deduplicate] [--drop-development-overlap] \
                 | score <network> <fen-text|-> \
                 | train --data-dir DIR --output-dir DIR [--epochs N] [--batch-size N] [--rate F] \
                 [--rate-decay F] [--l2 F] [--seed N] [--lambda F] [--k F] [--threads N] \
                 [--init-network FILE] [--ema F] [--factorize BOOL]"
                .to_owned(),
        ),
    };
    match result {
        Ok(message) => {
            eprintln!("nnue-data: {message}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("nnue-data: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Reads until `buffer` is full or the input ends; returns the bytes read.
fn fill(reader: &mut impl Read, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
}

fn open(path: &str) -> Result<Box<dyn BufRead>, String> {
    if path == "-" {
        Ok(Box::new(BufReader::new(io::stdin())))
    } else {
        std::fs::File::open(path)
            .map(|file| Box::new(BufReader::new(file)) as Box<dyn BufRead>)
            .map_err(|error| format!("{path}: {error}"))
    }
}

fn lines(
    mut reader: impl BufRead,
    mut consume: impl FnMut(usize, &str) -> Result<(), String>,
) -> Result<usize, String> {
    let mut buffer = Vec::new();
    let mut line_number = 0;
    let mut count = 0;
    loop {
        buffer.clear();
        let bytes = reader
            .by_ref()
            .take(MAX_LINE_BYTES + 1)
            .read_until(b'\n', &mut buffer)
            .map_err(|error| format!("line {}: {error}", line_number + 1))?;
        if bytes == 0 {
            break;
        }
        line_number += 1;
        if bytes as u64 > MAX_LINE_BYTES {
            return Err(format!(
                "line {line_number}: record exceeds {MAX_LINE_BYTES} bytes"
            ));
        }
        let line = std::str::from_utf8(&buffer)
            .map_err(|_| format!("line {line_number}: invalid UTF-8"))?
            .trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        consume(line_number, line).map_err(|error| format!("line {line_number}: {error}"))?;
        count += 1;
    }
    if count == 0 {
        return Err("input contains no positions".to_owned());
    }
    Ok(count)
}

fn canonical(board: &Board) -> String {
    let fen = board.to_string();
    let mut fields = fen.split_whitespace().take(4).collect::<Vec<_>>();
    if board.en_passant().is_some() {
        let mut legal_ep = false;
        board.generate_moves(|moves| {
            if board.piece_on(moves.from) == Some(Piece::Pawn) {
                for chess_move in moves {
                    legal_ep |= chess_move.from.file() != chess_move.to.file()
                        && board.piece_on(chess_move.to).is_none();
                }
            }
            legal_ep
        });
        if !legal_ep {
            fields[3] = "-";
        }
    }
    fields.join(" ")
}

fn dead_material(board: &Board) -> bool {
    if !(board.pieces(Piece::Pawn) | board.pieces(Piece::Rook) | board.pieces(Piece::Queen))
        .is_empty()
    {
        return false;
    }
    let knights = board.pieces(Piece::Knight);
    let bishops = board.pieces(Piece::Bishop);
    knights.len() + bishops.len() <= 1
        || (knights.is_empty()
            && ((bishops & BitBoard::DARK_SQUARES) == bishops
                || (bishops & BitBoard::LIGHT_SQUARES) == bishops))
}

struct Sample {
    board: Board,
    outcome: f64,
    teacher: Option<f64>,
}

/// Positions a static evaluation is not accountable for: decided, claimable
/// draws and dead material. Self-play corpora contain a few, since the arbiter
/// adjudicates only the standard insufficient-material cases.
fn is_terminal(board: &Board) -> bool {
    board.status() != GameStatus::Ongoing || board.halfmove_clock() >= 100 || dead_material(board)
}

fn parse_sample(line: &str) -> Result<Sample, String> {
    let fields = line.split(';').map(str::trim).collect::<Vec<_>>();
    if !(2..=3).contains(&fields.len()) {
        return Err("expected FEN;white-outcome[;white-score-cp]".to_owned());
    }
    let board = fields[0]
        .parse::<Board>()
        .map_err(|_| "invalid FEN".to_owned())?;
    let outcome = fields[1]
        .parse::<f64>()
        .map_err(|_| "invalid outcome".to_owned())?;
    if ![0.0, 0.5, 1.0].contains(&outcome) {
        return Err("white outcome must be 0, 0.5 or 1".to_owned());
    }
    let teacher = fields
        .get(2)
        .map(|score| {
            let score = score
                .parse::<f64>()
                .map_err(|_| "invalid teacher score".to_owned())?;
            if !score.is_finite() || score.abs() > 32_000.0 {
                return Err("teacher score must be finite centipawns in -32000..32000".to_owned());
            }
            Ok(score)
        })
        .transpose()?;
    Ok(Sample {
        board,
        outcome,
        teacher,
    })
}

fn comma(values: impl IntoIterator<Item = impl ToString>) -> String {
    values
        .into_iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

fn score(network: &Network, reader: impl BufRead, mut writer: impl Write) -> Result<usize, String> {
    let count = lines(reader, |_, line| {
        let board = line
            .parse::<Board>()
            .map_err(|_| "invalid FEN".to_owned())?;
        let accumulator = network.accumulator(&board);
        writeln!(
            writer,
            "{}\t{}\t{}",
            accumulator.evaluate(),
            comma(accumulator.values(Color::White)),
            comma(accumulator.values(Color::Black))
        )
        .map_err(|error| error.to_string())
    })?;
    writer.flush().map_err(|error| error.to_string())?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_and_dead_material_samples_are_recognized() {
        for fen in [
            "7k/6Q1/6K1/8/8/8/8/8 b - - 0 1",
            "7k/5K2/6Q1/8/8/8/8/8 b - - 0 1",
            "7k/8/8/8/8/8/8/KB6 w - - 0 1",
            "7k/8/8/8/8/8/8/KN6 w - - 0 1",
            "7k/8/8/8/8/8/8/K1B1B3 w - - 0 1",
            "4k3/8/5n2/8/8/8/2P5/4K3 w - - 100 70",
        ] {
            assert!(is_terminal(&fen.parse().unwrap()), "{fen}");
        }
        assert!(!is_terminal(
            &"7k/8/8/8/8/8/8/KNN5 w - - 0 1".parse().unwrap()
        ));
    }

    #[test]
    fn schema_lines_name_the_feature_contract_and_accept_version_one_datasets() {
        assert_eq!(*SCHEMA, "# jakgro-nnue-data-v2\t6144\t1");
        assert!(HEADER.starts_with(&format!("{}\n{COLUMNS}\n", *SCHEMA)));
        assert!(schema_accepted(&SCHEMA));
        // Version 1 headers carried the writer's hidden width, activation
        // ceiling and output scale; rows never depended on them.
        assert!(schema_accepted("# jakgro-nnue-data-v1\t6144\t128\t255\t64"));
        assert!(schema_accepted("# jakgro-nnue-data-v1\t6144\t512\t255\t64"));
        for rejected in [
            "# jakgro-nnue-data-v1\t12288\t128\t255\t64",
            "# jakgro-nnue-data-v1\t6144",
            "# jakgro-nnue-data-v1\t6144\t128\t255",
            "# jakgro-nnue-data-v2\t6144\t2",
            "# jakgro-nnue-data-v2\t6144\t1\t512",
            "# jakgro-nnue-data-v3\t6144\t1",
            "",
        ] {
            assert!(!schema_accepted(rejected), "{rejected:?}");
        }
    }

    const FEN: &str = "4k3/8/5n2/8/8/8/2P5/4K3 w - - 0 1";

    #[test]
    fn black_labels_are_preserved_as_white_relative() {
        let black = FEN.replace(" w ", " b ");
        let sample = parse_sample(&format!("{black};1;345")).unwrap();
        assert_eq!(sample.outcome, 1.0);
        assert_eq!(sample.teacher, Some(345.0));
    }

    fn consume_all(input: &[u8]) -> Result<usize, String> {
        lines(input, |_, line| parse_sample(line).map(|_| ()))
    }

    #[test]
    fn malformed_labels_are_not_silently_dropped() {
        for fields in [
            "", ";", ";NaN", ";2", ";0.7", ";1;NaN", ";1;inf", ";1;32001", ";1;3;4",
        ] {
            assert!(parse_sample(&format!("{FEN}{fields}")).is_err(), "{fields}");
        }
        assert!(parse_sample("invalid;1").is_err());
        // A bare-kings position parses; preparation drops it as terminal.
        assert!(is_terminal(
            &parse_sample("7k/8/8/8/8/8/8/K7 w - - 0 1;0.5")
                .unwrap()
                .board
        ));
        let input = format!("{FEN};1\n{FEN};NaN\n");
        assert!(
            consume_all(input.as_bytes())
                .unwrap_err()
                .contains("line 2")
        );
    }

    #[test]
    fn line_lengths_encoding_and_empty_inputs_are_bounded() {
        assert!(consume_all("# empty\n".as_bytes()).is_err());
        assert!(
            consume_all([0xff, b'\n'].as_slice())
                .unwrap_err()
                .contains("UTF-8")
        );
        assert!(
            consume_all(vec![b'a'; 5000].as_slice())
                .unwrap_err()
                .contains("exceeds")
        );
    }

    #[test]
    fn canonical_keys_ignore_clocks_and_ineffective_en_passant() {
        let first: Board = "4k3/8/8/3p4/8/8/8/4K3 w - d6 0 1".parse().unwrap();
        let second: Board = "4k3/8/8/3p4/8/8/8/4K3 w - - 70 90".parse().unwrap();
        assert_eq!(canonical(&first), canonical(&second));
        let legal: Board = "4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1".parse().unwrap();
        assert!(canonical(&legal).ends_with(" d6"));
    }
}
