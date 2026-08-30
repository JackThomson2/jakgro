//! Offline fitter for the objective evaluation weights.
//!
//! The evaluation is a linear model over the vector `jakgro::engine::tuning`
//! exposes, so fitting it is ordinary logistic regression against game results:
//! find the weights under which the engine's own score best predicts who won.
//! This is the method Texel's tuning popularised, and the only thing unusual
//! here is what is *excluded* from it — the attacking-style weights and the
//! profile mobility adjustment are never touched, because game results do not
//! reward interesting chess and an optimiser handed them would quietly tune the
//! engine's character out of it.
//!
//! Two subcommands, so the expensive step runs once:
//!
//! ```text
//! tune extract --pgn <file>... --out positions.txt [--skip-plies N]
//! tune fit --positions positions.txt --out weights.txt [--epochs N] [--seed N]
//! ```
//!
//! `extract` replays games and writes the quiet positions worth learning from,
//! labelled by the result of the game they came from. `fit` reads those, turns
//! each into a feature vector once, and runs Adam over the whole corpus.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::fs;
use std::process::ExitCode;

use cozy_chess::{Board, Piece};
use jakgro::engine::tuning::{
    FEATURE_COUNT, MAX_PHASE, PLACEMENT_OFFSET, SCALAR_FEATURES, TuningPosition, current_weights,
    normalized, tuning_features,
};

/// Scaling constant relating a centipawn score to a winning probability.
///
/// The conventional Elo-style form, so a fitted `K` is comparable with the value
/// other engines report.
fn winning_probability(score: f64, k: f64) -> f64 {
    1.0 / (1.0 + 10_f64.powf(-k * score / 400.0))
}

/// One labelled position: its features and the result of the game it came from.
struct Sample {
    entries: Vec<(u16, i16)>,
    /// Middlegame share of the blend, already divided by the phase maximum.
    middle_game_share: f64,
    /// Result from White's perspective: 1.0, 0.5 or 0.0.
    outcome: f64,
}

impl Sample {
    fn score(&self, weights: &[f64]) -> f64 {
        let mut middle_game = 0.0;
        let mut end_game = 0.0;
        for &(index, count) in &self.entries {
            let count = f64::from(count);
            middle_game += weights[index as usize] * count;
            end_game += weights[FEATURE_COUNT + index as usize] * count;
        }
        middle_game * self.middle_game_share + end_game * (1.0 - self.middle_game_share)
    }
}

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let result = match arguments.first().map(String::as_str) {
        Some("extract") => extract(&arguments[1..]),
        Some("fit") => fit(&arguments[1..]),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::FAILURE;
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("tune: {error}");
            ExitCode::FAILURE
        }
    }
}

const USAGE: &str = "\
usage:
  tune extract --pgn <file>... --out <positions> [--skip-plies N] [--max-positions N]
  tune fit --positions <file> --out <weights> [--epochs N] [--rate F] [--holdout F]";

/// Returns the value following a flag, if the flag is present.
fn flag<'a>(arguments: &'a [String], name: &str) -> Option<&'a str> {
    arguments
        .iter()
        .position(|argument| argument == name)
        .and_then(|index| arguments.get(index + 1))
        .map(String::as_str)
}

fn parse_flag<T: std::str::FromStr>(
    arguments: &[String],
    name: &str,
    fallback: T,
) -> Result<T, String> {
    match flag(arguments, name) {
        None => Ok(fallback),
        Some(value) => value
            .parse()
            .map_err(|_| format!("{name} expects a value, got {value:?}")),
    }
}

// ---------------------------------------------------------------- extraction

fn extract(arguments: &[String]) -> Result<(), String> {
    let out = flag(arguments, "--out").ok_or("extract needs --out")?;
    let skip_plies: usize = parse_flag(arguments, "--skip-plies", 8)?;
    let max_positions: usize = parse_flag(arguments, "--max-positions", usize::MAX)?;
    let pgns = list_after(arguments, "--pgn");
    if pgns.is_empty() {
        return Err("extract needs at least one --pgn file".into());
    }

    let mut seen = HashSet::new();
    let mut written = String::new();
    let mut games = 0_usize;
    let mut kept = 0_usize;

    for path in &pgns {
        let text = fs::read_to_string(path).map_err(|error| format!("{path}: {error}"))?;
        for game in split_games(&text) {
            games += 1;
            let Some(outcome) = game.outcome else {
                continue;
            };
            let Ok(mut board) = game.start.parse::<Board>() else {
                continue;
            };
            for (ply, san) in game.moves.iter().enumerate() {
                let Ok(chess_move) = cozy_chess::util::parse_san_move(&board, san) else {
                    break;
                };
                if ply >= skip_plies && is_learnable(&board, chess_move) {
                    let fen = format!("{board}");
                    if seen.insert(fen.clone()) {
                        let _ = writeln!(written, "{fen};{outcome}");
                        kept += 1;
                        if kept >= max_positions {
                            break;
                        }
                    }
                }
                board.play_unchecked(chess_move);
            }
            if kept >= max_positions {
                break;
            }
        }
    }

    fs::write(out, &written).map_err(|error| format!("{out}: {error}"))?;
    println!("extract: {games} games, {kept} positions written to {out}");
    Ok(())
}

/// Reports whether a position teaches the evaluation anything.
///
/// The evaluation is static, so it is only accountable for positions where a
/// static judgement is meaningful. A position in check is settled by search, and
/// one whose best continuation wins material is about to be worth something
/// quite different from what it looks like now; both would teach the model to
/// account for tactics it cannot see, and a tuner that learns those pushes every
/// weight toward explaining noise.
fn is_learnable(board: &Board, chess_move: cozy_chess::Move) -> bool {
    board.checkers().is_empty()
        && chess_move.promotion.is_none()
        && board.color_on(chess_move.to) != Some(!board.side_to_move())
}

struct Game {
    start: String,
    moves: Vec<String>,
    outcome: Option<f64>,
}

const START_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

/// Splits a PGN file into games, keeping only what a replay needs.
fn split_games(text: &str) -> Vec<Game> {
    let mut games = Vec::new();
    let mut start = START_FEN.to_owned();
    let mut outcome = None;
    let mut movetext = String::new();
    let mut in_moves = false;

    for line in text.lines() {
        let line = line.trim();
        if let Some(tag) = line.strip_prefix('[') {
            if in_moves {
                games.push(finish_game(&start, &movetext, outcome));
                start = START_FEN.to_owned();
                outcome = None;
                movetext.clear();
                in_moves = false;
            }
            if let Some(value) = tag_value(tag, "FEN") {
                start = value;
            } else if let Some(value) = tag_value(tag, "Result") {
                outcome = result_value(&value);
            }
            continue;
        }
        if line.is_empty() {
            continue;
        }
        in_moves = true;
        movetext.push(' ');
        movetext.push_str(line);
    }
    if in_moves {
        games.push(finish_game(&start, &movetext, outcome));
    }
    games
}

fn finish_game(start: &str, movetext: &str, outcome: Option<f64>) -> Game {
    Game {
        start: start.to_owned(),
        moves: movetext
            .split_whitespace()
            .filter(|token| {
                !token.is_empty()
                    && !token.ends_with('.')
                    && !token.starts_with('$')
                    && result_value(token).is_none()
                    && token.chars().next().is_some_and(char::is_alphabetic)
            })
            .map(str::to_owned)
            .collect(),
        outcome,
    }
}

fn tag_value(tag: &str, name: &str) -> Option<String> {
    let rest = tag.strip_prefix(name)?.trim_start();
    let inner = rest.strip_prefix('"')?;
    let end = inner.find('"')?;
    Some(inner[..end].to_owned())
}

fn result_value(token: &str) -> Option<f64> {
    match token.trim_end_matches(']').trim_matches('"') {
        "1-0" => Some(1.0),
        "0-1" => Some(0.0),
        "1/2-1/2" => Some(0.5),
        _ => None,
    }
}

fn list_after(arguments: &[String], name: &str) -> Vec<String> {
    let Some(index) = arguments.iter().position(|argument| argument == name) else {
        return Vec::new();
    };
    arguments[index + 1..]
        .iter()
        .take_while(|argument| !argument.starts_with("--"))
        .cloned()
        .collect()
}

// ------------------------------------------------------------------- fitting

fn fit(arguments: &[String]) -> Result<(), String> {
    let positions = flag(arguments, "--positions").ok_or("fit needs --positions")?;
    let out = flag(arguments, "--out").ok_or("fit needs --out")?;
    let epochs: usize = parse_flag(arguments, "--epochs", 400)?;
    let rate: f64 = parse_flag(arguments, "--rate", 1.0)?;
    let holdout: f64 = parse_flag(arguments, "--holdout", 0.1)?;
    let l2: f64 = parse_flag(arguments, "--l2", 2e-4)?;
    let min_observations: u64 = parse_flag(arguments, "--min-observations", 2_000)?;

    let text = fs::read_to_string(positions).map_err(|error| format!("{positions}: {error}"))?;
    let mut samples = Vec::new();
    for line in text.lines() {
        let Some((fen, outcome)) = line.rsplit_once(';') else {
            continue;
        };
        let (Ok(board), Ok(outcome)) = (fen.parse::<Board>(), outcome.parse::<f64>()) else {
            continue;
        };
        let vector = tuning_features(&board);
        samples.push(Sample {
            entries: vector.entries,
            middle_game_share: f64::from(vector.phase) / f64::from(MAX_PHASE),
            outcome,
        });
    }
    if samples.is_empty() {
        return Err(format!("{positions} yielded no usable positions"));
    }

    // A deterministic split: every tenth position is held out, so the reported
    // held-out loss is reproducible without shuffling a million records.
    let holdout_stride = if holdout > 0.0 {
        (1.0 / holdout).round().max(2.0) as usize
    } else {
        usize::MAX
    };
    let mut training: Vec<&Sample> = Vec::new();
    let mut held_out: Vec<&Sample> = Vec::new();
    for (index, sample) in samples.iter().enumerate() {
        if holdout_stride != usize::MAX && index % holdout_stride == 0 {
            held_out.push(sample);
        } else {
            training.push(sample);
        }
    }

    let start = current_weights();
    let mut weights = vec![0.0_f64; 2 * FEATURE_COUNT];
    for (index, &(middle_game, end_game)) in start.iter().enumerate() {
        weights[index] = f64::from(middle_game);
        weights[FEATURE_COUNT + index] = f64::from(end_game);
    }

    // Count what the corpus actually supports. Adam normalises each parameter by
    // its own gradient magnitude, so a feature seen a handful of times takes
    // steps exactly as large as one seen a million times, and its weight walks
    // off to wherever the noise points. A king on the eighth rank is the clearest
    // case: White's king is almost never there, so nothing anchors those squares.
    let mut observations = vec![0_u64; FEATURE_COUNT];
    for sample in &samples {
        for &(index, _) in &sample.entries {
            observations[index as usize] += 1;
        }
    }
    let anchored: Vec<bool> = observations
        .iter()
        .map(|&count| count < min_observations)
        .collect();
    let pinned = anchored.iter().filter(|&&is_pinned| is_pinned).count();

    let start_weights = weights.clone();
    let k = fit_scaling(&training, &weights);
    println!(
        "fit: {} positions ({} training, {} held out), K = {k:.4}",
        samples.len(),
        training.len(),
        held_out.len(),
    );
    println!(
        "fit: L2 {l2:e} toward the published weights, {pinned} of {FEATURE_COUNT} features held \
         there for fewer than {min_observations} observations",
    );
    println!(
        "fit: starting loss {:.6} training, {:.6} held out",
        loss(&training, &weights, k),
        loss(&held_out, &weights, k),
    );

    adam(
        &training,
        &mut weights,
        &start_weights,
        &anchored,
        k,
        epochs,
        rate,
        l2,
    );

    println!(
        "fit: final loss    {:.6} training, {:.6} held out",
        loss(&training, &weights, k),
        loss(&held_out, &weights, k),
    );

    let rounded: Vec<(i32, i32)> = (0..FEATURE_COUNT)
        .map(|index| {
            (
                weights[index].round() as i32,
                weights[FEATURE_COUNT + index].round() as i32,
            )
        })
        .collect();
    let final_weights = normalized(&rounded);
    verify_round_trip(&samples, &rounded, &final_weights);

    fs::write(out, render_source(&final_weights)).map_err(|error| format!("{out}: {error}"))?;
    println!("fit: wrote {out}");
    Ok(())
}

/// Chooses the scaling constant that best relates the current scores to results.
///
/// A ternary search over a unimodal curve, which is what the loss is in `K`.
fn fit_scaling(samples: &[&Sample], weights: &[f64]) -> f64 {
    let (mut low, mut high) = (0.1_f64, 10.0_f64);
    for _ in 0..60 {
        let first = low + (high - low) / 3.0;
        let second = high - (high - low) / 3.0;
        if loss(samples, weights, first) < loss(samples, weights, second) {
            high = second;
        } else {
            low = first;
        }
    }
    (low + high) / 2.0
}

fn loss(samples: &[&Sample], weights: &[f64], k: f64) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let total: f64 = samples
        .iter()
        .map(|sample| {
            let error = sample.outcome - winning_probability(sample.score(weights), k);
            error * error
        })
        .sum();
    total / samples.len() as f64
}

/// Full-batch Adam over the whole corpus.
///
/// Full batch rather than stochastic because the gradient is sparse and cheap:
/// each position touches about thirty of the eight hundred parameters, so a pass
/// costs little more than reading the corpus, and a deterministic gradient makes
/// the fit reproducible without a seeded shuffle.
#[allow(clippy::too_many_arguments)]
fn adam(
    samples: &[&Sample],
    weights: &mut [f64],
    start: &[f64],
    anchored: &[bool],
    k: f64,
    epochs: usize,
    rate: f64,
    l2: f64,
) {
    const BETA1: f64 = 0.9;
    const BETA2: f64 = 0.999;
    const EPSILON: f64 = 1e-8;
    let scale = k * std::f64::consts::LN_10 / 400.0;

    let mut moment = vec![0.0_f64; weights.len()];
    let mut velocity = vec![0.0_f64; weights.len()];
    let count = samples.len() as f64;

    for epoch in 1..=epochs {
        let mut gradient = vec![0.0_f64; weights.len()];
        for sample in samples {
            let probability = winning_probability(sample.score(weights), k);
            // d/ds of (y - sigma(s))^2, with the sigmoid's own derivative folded in.
            let outer =
                -2.0 * (sample.outcome - probability) * probability * (1.0 - probability) * scale;
            let middle_game_share = sample.middle_game_share;
            for &(index, value) in &sample.entries {
                let value = f64::from(value);
                gradient[index as usize] += outer * value * middle_game_share;
                gradient[FEATURE_COUNT + index as usize] +=
                    outer * value * (1.0 - middle_game_share);
            }
        }

        let correction1 = 1.0 - BETA1.powi(epoch as i32);
        let correction2 = 1.0 - BETA2.powi(epoch as i32);
        for index in 0..weights.len() {
            if anchored[index % FEATURE_COUNT] {
                weights[index] = start[index];
                continue;
            }
            // Pulling toward the published value rather than toward zero. The
            // published weights are a working evaluation, so this says "move
            // where the data insists, and stay put where it is indifferent",
            // which is the shape of the evidence a self-play corpus provides.
            let derivative = gradient[index] / count + l2 * (weights[index] - start[index]);
            moment[index] = BETA1 * moment[index] + (1.0 - BETA1) * derivative;
            velocity[index] = BETA2 * velocity[index] + (1.0 - BETA2) * derivative * derivative;
            let step = rate * (moment[index] / correction1)
                / ((velocity[index] / correction2).sqrt() + EPSILON);
            weights[index] -= step;
        }

        if epoch % 50 == 0 || epoch == epochs {
            println!("  epoch {epoch:>4}: loss {:.6}", loss(samples, weights, k));
        }
    }
}

/// Checks that re-centring did not move any score before the tables are written.
fn verify_round_trip(samples: &[Sample], before: &[(i32, i32)], after: &[(i32, i32)]) {
    let mut worst = 0;
    for sample in samples.iter().take(20_000) {
        let position = TuningPosition {
            entries: sample.entries.clone(),
            phase: (sample.middle_game_share * f64::from(MAX_PHASE)).round() as i32,
        };
        worst = worst.max((position.score(before) - position.score(after)).abs());
    }
    assert!(
        worst <= 1,
        "re-centring moved a score by {worst} centipawns, which it must not",
    );
}

// ------------------------------------------------------------------ emission

fn render_source(weights: &[(i32, i32)]) -> String {
    let mut out = String::new();
    out.push_str("// Fitted by `tune fit`. Paste into the files named below.\n\n");
    out.push_str("// ---- src/engine/evaluation/weights.rs ----\n");
    for (index, name) in SCALAR_NAMES.iter().enumerate() {
        let (middle_game, end_game) = weights[index];
        let _ = writeln!(
            out,
            "const {name}: ScorePair = ScorePair::new({middle_game}, {end_game});"
        );
    }
    out.push_str("\n// ---- src/engine/evaluation/placement.rs ----\n");
    for piece in Piece::ALL {
        let name = match piece {
            Piece::Pawn => "PAWN",
            Piece::Knight => "KNIGHT",
            Piece::Bishop => "BISHOP",
            Piece::Rook => "ROOK",
            Piece::Queen => "QUEEN",
            Piece::King => "KING",
        };
        let base = PLACEMENT_OFFSET + piece as usize * 64;
        let _ = writeln!(out, "static {name}: Table = Table {{");
        for (label, pick) in [("middle_game", 0_usize), ("end_game", 1)] {
            let _ = writeln!(out, "    {label}: [");
            for rank in 0..8 {
                let row: Vec<String> = (0..8)
                    .map(|file| {
                        let entry = weights[base + rank * 8 + file];
                        let value = if pick == 0 { entry.0 } else { entry.1 };
                        value.to_string()
                    })
                    .collect();
                let _ = writeln!(out, "        {}, //", row.join(", "));
            }
            let _ = writeln!(out, "    ],");
        }
        out.push_str("};\n");
    }
    out
}

const SCALAR_NAMES: [&str; SCALAR_FEATURES] = [
    "PAWN",
    "KNIGHT",
    "BISHOP",
    "ROOK",
    "QUEEN",
    "ACTIVITY",
    "TEMPO",
    "MOBILITY",
    "BISHOP_PAIR",
    "DOUBLED_PAWN",
    "ISOLATED_PAWN",
    "PASSED_PAWN",
    "KING_SHELTER",
    "OPEN_KING_FILE",
];
