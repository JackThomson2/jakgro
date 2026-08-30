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
//! tune fit --positions positions.txt --out weights.txt [--epochs N] [--lambda F]
//! ```
//!
//! `extract` replays games and writes the quiet positions worth learning from,
//! labelled by the result of the game they came from and, where the PGN records
//! one, by the score the engine reported when it moved. `fit` reads those, turns
//! each into a feature vector once, and runs Adam over the whole corpus.
//!
//! The label `fit` optimises against is
//!
//! ```text
//! label = lambda * outcome + (1 - lambda) * sigmoid(score / K)
//! ```
//!
//! A game result is a very noisy label for a single position, because a won game
//! is full of positions that were not winning; the engine's own estimate is less
//! noisy but only as good as the evaluation being fitted. `--lambda` picks the
//! mixture and defaults to 1.0, the game result alone, which is what every fit
//! before this one used. The score is kept in the corpus rather than folded into
//! a label at extraction time so the mixture can be screened without paying for
//! extraction again.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::fs;
use std::process::ExitCode;

use cozy_chess::Board;
use jakgro::engine::tuning::{
    BLOCKS, BlockKind, FEATURE_COUNT, FeatureBlock, MAX_PHASE, TuningPosition, anchored,
    current_weights, normalized, tuning_features,
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
    /// White-relative centipawns the engine reported here, where recorded.
    search_score: Option<f64>,
    /// The label actually fitted against, once `--lambda` has been applied.
    label: f64,
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
  tune fit --positions <file> --out <weights> [--epochs N] [--rate F] [--holdout F]
                                              [--lambda F] [--l2 F] [--min-observations N]";

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
    let mut scored = 0_usize;

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
                        // Features are White-positive, so the mover-relative
                        // score the PGN carries is flipped to match before it is
                        // ever compared with a weight.
                        let score = game.scores.get(ply).copied().flatten().map(|score| {
                            if board.side_to_move() == cozy_chess::Color::White {
                                score
                            } else {
                                -score
                            }
                        });
                        match score {
                            Some(score) => {
                                let _ = writeln!(written, "{fen};{outcome};{score}");
                                scored += 1;
                            }
                            None => {
                                let _ = writeln!(written, "{fen};{outcome}");
                            }
                        }
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
    println!(
        "extract: {games} games, {kept} positions written to {out}, {scored} carrying a search \
         score"
    );
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
    /// Centipawns the mover reported for each ply, where the PGN annotated it.
    scores: Vec<Option<f64>>,
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
    let mut moves: Vec<String> = Vec::new();
    let mut scores: Vec<Option<f64>> = Vec::new();
    let mut token = String::new();
    let mut comment = String::new();
    let mut in_comment = false;

    // Comments are attached to the move they follow, so the scan cannot be a
    // `split_whitespace` filter: a `{...}` may contain spaces, and dropping it
    // token by token would lose which move it described.
    let flush = |token: &mut String, moves: &mut Vec<String>, scores: &mut Vec<_>| {
        if is_move_token(token) {
            moves.push(std::mem::take(token));
            scores.push(None);
        } else {
            token.clear();
        }
    };
    for character in movetext.chars() {
        if in_comment {
            if character == '}' {
                in_comment = false;
                if let Some(last) = scores.last_mut() {
                    *last = comment_score(&comment);
                }
                comment.clear();
            } else {
                comment.push(character);
            }
            continue;
        }
        match character {
            '{' => {
                flush(&mut token, &mut moves, &mut scores);
                in_comment = true;
            }
            character if character.is_whitespace() => {
                flush(&mut token, &mut moves, &mut scores);
            }
            character => token.push(character),
        }
    }
    flush(&mut token, &mut moves, &mut scores);

    Game {
        start: start.to_owned(),
        moves,
        scores,
        outcome,
    }
}

/// Reports whether a movetext token is a move rather than punctuation.
fn is_move_token(token: &str) -> bool {
    !token.is_empty()
        && !token.ends_with('.')
        && !token.starts_with('$')
        && result_value(token).is_none()
        && token.chars().next().is_some_and(char::is_alphabetic)
}

/// Extracts the centipawn score from a `{+0.31/12}` comment, if it carries one.
///
/// The comment is written in pawns from the mover's perspective, which is the
/// convention `cutechess-cli` and this repository's arbiter share. Anything else
/// inside the braces — a termination note, a clock reading — yields nothing
/// rather than a wrong number.
fn comment_score(comment: &str) -> Option<f64> {
    let text = comment.trim();
    let value = text.split('/').next()?.trim();
    if !value.starts_with('+') && !value.starts_with('-') {
        return None;
    }
    // Rounded because the source is centipawns rendered to two decimal places,
    // so the product is integral up to binary floating-point error, and writing
    // `-7.000000000000001` into the corpus helps nobody read it.
    value
        .parse::<f64>()
        .ok()
        .map(|pawns| (pawns * 100.0).round())
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
    // 1.0 is the game result alone, which is what every fit before this one
    // used. Lower values mix in the engine's own estimate for the position.
    let lambda: f64 = parse_flag(arguments, "--lambda", 1.0)?;
    if !(0.0..=1.0).contains(&lambda) {
        return Err(format!("--lambda expects a value in [0, 1], got {lambda}"));
    }

    let text = fs::read_to_string(positions).map_err(|error| format!("{positions}: {error}"))?;
    let mut samples = Vec::new();
    for line in text.lines() {
        // Two forms are accepted. `fen;outcome` is what every corpus written
        // before search scores were recorded contains, and stays readable;
        // `fen;outcome;score` carries the mover's own estimate as well.
        let mut fields = line.split(';');
        let (Some(fen), Some(outcome)) = (fields.next(), fields.next()) else {
            continue;
        };
        let (Ok(board), Ok(outcome)) = (fen.parse::<Board>(), outcome.parse::<f64>()) else {
            continue;
        };
        let search_score = fields
            .next()
            .and_then(|score| score.trim().parse::<f64>().ok());
        let vector = tuning_features(&board);
        samples.push(Sample {
            entries: vector.entries,
            middle_game_share: f64::from(vector.phase) / f64::from(MAX_PHASE),
            outcome,
            search_score,
            // Replaced below, once K is known.
            label: outcome,
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
    let is_held_out = |index: usize| holdout_stride != usize::MAX && index % holdout_stride == 0;

    let start = current_weights();
    let mut weights = vec![0.0_f64; 2 * FEATURE_COUNT];
    for (index, &(middle_game, end_game)) in start.iter().enumerate() {
        weights[index] = f64::from(middle_game);
        weights[FEATURE_COUNT + index] = f64::from(end_game);
    }

    // K is fitted against the game result alone, before any blending, for two
    // reasons. It keeps the constant comparable with the value other engines
    // report, which is the whole point of writing it in the Elo-style form; and
    // blending with a sigmoid whose K was itself fitted to the blended label
    // would be circular.
    let k = {
        let training: Vec<&Sample> = samples
            .iter()
            .enumerate()
            .filter(|(index, _)| !is_held_out(*index))
            .map(|(_, sample)| sample)
            .collect();
        fit_scaling(&training, &weights)
    };

    let scored = samples
        .iter()
        .filter(|sample| sample.search_score.is_some())
        .count();
    if lambda < 1.0 {
        for sample in &mut samples {
            // A position with no recorded score keeps the pure outcome label.
            // Blending it toward a score that does not exist would quietly
            // relabel part of the corpus as a draw.
            if let Some(score) = sample.search_score {
                sample.label =
                    lambda * sample.outcome + (1.0 - lambda) * winning_probability(score, k);
            }
        }
    }

    let mut training: Vec<&Sample> = Vec::new();
    let mut held_out: Vec<&Sample> = Vec::new();
    for (index, sample) in samples.iter().enumerate() {
        if is_held_out(index) {
            held_out.push(sample);
        } else {
            training.push(sample);
        }
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
    let held_at_published: Vec<bool> = observations
        .iter()
        .map(|&count| count < min_observations)
        .collect();
    let pinned = held_at_published
        .iter()
        .filter(|&&is_pinned| is_pinned)
        .count();

    let start_weights = weights.clone();
    println!(
        "fit: {} positions ({} training, {} held out), K = {k:.4}",
        samples.len(),
        training.len(),
        held_out.len(),
    );
    println!(
        "fit: lambda {lambda:.2}, {scored} of {} positions carrying a search score",
        samples.len(),
    );
    if lambda < 1.0 {
        // The label moved, so the loss is against a different target and is not
        // comparable with a run at another lambda. Only the improvement from
        // starting to final loss within one run means anything, and only a match
        // decides between two runs.
        println!(
            "fit: losses below are against the blended label and are not comparable \
             across lambda",
        );
    }
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
        &held_at_published,
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
    // Centre the tables, then pull the whole vector back onto the scale the
    // search's fixed margins were measured against. The fit is invariant under a
    // positive scale, so without the anchor each refit quietly re-calibrates
    // every futility margin, the swap-list piece values, and the personality's
    // style cap.
    let centred = normalized(&rounded);
    if centred[0].0 <= 0 {
        // Anchoring cannot rescue this and must not try: dividing by a
        // non-positive pawn would flip the sign of every weight. A fit that
        // decided a pawn is worth nothing has failed, and an unregularised run
        // on a small corpus reaches -9 readily, so this is a real outcome rather
        // than a defensive impossibility.
        return Err(format!(
            "the fit put the middlegame pawn at {}, so the scale is meaningless; \
             raise --l2 or --min-observations, or fit a larger corpus",
            centred[0].0,
        ));
    }
    let final_weights = anchored(&centred);
    println!(
        "fit: middlegame pawn {} fitted, {} after anchoring",
        centred[0].0, final_weights[0].0,
    );

    // The two steps guarantee different things and are checked separately.
    // Re-centring moves weight between the tables and the material values and
    // must not move a score at all. Anchoring scales every score on purpose, so
    // what it must preserve is the order.
    verify_round_trip(&samples, &rounded, &centred);
    verify_anchor_preserves_order(&samples, &centred, &final_weights);

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
            let error = sample.label - winning_probability(sample.score(weights), k);
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
    held_at_published: &[bool],
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
                -2.0 * (sample.label - probability) * probability * (1.0 - probability) * scale;
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
            if held_at_published[index % FEATURE_COUNT] {
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

/// Checks that anchoring rescaled every score without reordering any two.
///
/// Anchoring multiplies the whole vector by a constant, so scores change and
/// must; what cannot change is which of two positions the evaluation prefers.
/// Rounding each weight to an integer is what makes this worth checking rather
/// than assuming: a genuine uniform scale could not reorder anything, but a
/// rounded one can, and only for positions the evaluation already considers
/// near-equal.
fn verify_anchor_preserves_order(samples: &[Sample], before: &[(i32, i32)], after: &[(i32, i32)]) {
    let scored: Vec<(i32, i32)> = samples
        .iter()
        .take(4_000)
        .map(|sample| {
            let position = TuningPosition {
                entries: sample.entries.clone(),
                phase: (sample.middle_game_share * f64::from(MAX_PHASE)).round() as i32,
            };
            (position.score(before), position.score(after))
        })
        .collect();

    let mut inversions = 0_usize;
    let mut comparisons = 0_usize;
    for (index, left) in scored.iter().enumerate() {
        for right in &scored[index + 1..] {
            comparisons += 1;
            if left.0.cmp(&right.0) != left.1.cmp(&right.1) {
                inversions += 1;
            }
        }
    }

    // A handful of ties broken differently is rounding, not a defect. A
    // meaningful share would mean the anchor ratio is extreme enough that
    // integer weights can no longer represent the fit.
    let limit = comparisons / 1_000;
    assert!(
        inversions <= limit,
        "anchoring reordered {inversions} of {comparisons} position pairs, above the \
         {limit} rounding allows",
    );
}

// ------------------------------------------------------------------ emission

fn render_source(weights: &[(i32, i32)]) -> String {
    render_blocks(weights, BLOCKS)
}

/// Renders one weight vector against a given layout.
///
/// Taking the blocks as an argument rather than reading the constant keeps the
/// emitter testable against a small synthetic layout, which is how the array
/// form is covered before any real feature group uses it.
fn render_blocks(weights: &[(i32, i32)], blocks: &[FeatureBlock]) -> String {
    let mut out = String::new();
    out.push_str("// Fitted by `tune fit`. Paste into the files named below.\n\n");
    out.push_str("// ---- src/engine/evaluation/weights.rs ----\n");
    let mut in_tables = false;
    for block in blocks {
        if block.kind == BlockKind::Table && !in_tables {
            out.push_str("\n// ---- src/engine/evaluation/placement.rs ----\n");
            in_tables = true;
        }
        match block.kind {
            BlockKind::Scalar => {
                let (middle_game, end_game) = weights[block.offset];
                let _ = writeln!(
                    out,
                    "const {}: ScorePair = ScorePair::new({middle_game}, {end_game});",
                    block.name,
                );
            }
            BlockKind::Array => {
                let entries: Vec<String> = (0..block.len)
                    .map(|index| {
                        let (middle_game, end_game) = weights[block.offset + index];
                        format!("ScorePair::new({middle_game}, {end_game})")
                    })
                    .collect();
                let _ = writeln!(
                    out,
                    "const {}: [ScorePair; {}] = [\n    {},\n];",
                    block.name,
                    block.len,
                    entries.join(",\n    "),
                );
            }
            BlockKind::Table => {
                let _ = writeln!(out, "static {}: Table = Table {{", block.name);
                for (label, pick) in [("middle_game", 0_usize), ("end_game", 1)] {
                    let _ = writeln!(out, "    {label}: [");
                    for rank in 0..8 {
                        let row: Vec<String> = (0..8)
                            .map(|file| {
                                let entry = weights[block.offset + rank * 8 + file];
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
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{FeatureBlock, comment_score, render_blocks, split_games, winning_probability};
    use jakgro::engine::tuning::BlockKind;

    /// A PGN in the form `selfplay` now writes, wrapped mid-comment.
    ///
    /// The wrap is deliberate. Movetext is folded at 79 columns without regard
    /// for comment boundaries, so a scan that assumed a comment lived on one
    /// line would mis-associate every score after the first long game.
    const ANNOTATED: &str = "\
[FEN \"rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1\"]
[Result \"1-0\"]

1. e4 {+0.31/12} e5 {-0.08/11} 2. Nf3 {+0.35/12}
Nc6 {-0.11/12} 1-0
";

    /// The array form has no user yet, so it is covered against a synthetic
    /// layout rather than waiting for the first feature group to rely on it.
    #[test]
    fn each_block_kind_renders_the_constant_it_names() {
        let blocks = [
            FeatureBlock {
                name: "TEMPO",
                offset: 0,
                len: 1,
                kind: BlockKind::Scalar,
            },
            FeatureBlock {
                name: "PASSED_PAWN_BY_RANK",
                offset: 1,
                len: 3,
                kind: BlockKind::Array,
            },
        ];
        let weights = [(12, 0), (5, 16), (20, 40), (60, 120)];

        let rendered = render_blocks(&weights, &blocks);

        assert!(rendered.contains("const TEMPO: ScorePair = ScorePair::new(12, 0);"));
        assert!(rendered.contains("const PASSED_PAWN_BY_RANK: [ScorePair; 3] = ["));
        assert!(rendered.contains("ScorePair::new(5, 16),"));
        assert!(rendered.contains("ScorePair::new(60, 120),"));
        // No table block, so the placement banner must not be emitted.
        assert!(!rendered.contains("placement.rs"));
    }

    #[test]
    fn comments_are_paired_with_the_move_they_follow() {
        let games = split_games(ANNOTATED);

        assert_eq!(games.len(), 1);
        let game = &games[0];
        assert_eq!(game.moves, ["e4", "e5", "Nf3", "Nc6"]);
        assert_eq!(
            game.scores,
            [Some(31.0), Some(-8.0), Some(35.0), Some(-11.0)]
        );
        assert_eq!(game.outcome, Some(1.0));
    }

    #[test]
    fn an_unannotated_game_still_parses_with_no_scores() {
        let games = split_games("[Result \"1/2-1/2\"]\n\n1. e4 e5 2. Nf3 Nc6 1/2-1/2\n");

        assert_eq!(games.len(), 1);
        assert_eq!(games[0].moves, ["e4", "e5", "Nf3", "Nc6"]);
        assert_eq!(games[0].scores, [None, None, None, None]);
    }

    #[test]
    fn only_a_signed_leading_value_is_read_as_a_score() {
        assert_eq!(comment_score("+0.31/12"), Some(31.0));
        assert_eq!(comment_score("-1.50/8"), Some(-150.0));
        assert_eq!(comment_score(" +0.00/1 "), Some(0.0));
        // An unsigned number is not the convention and is more likely to be a
        // clock reading than an evaluation, so it is refused rather than
        // guessed at.
        assert_eq!(comment_score("0.31/12"), None);
        assert_eq!(comment_score("book"), None);
        assert_eq!(comment_score(""), None);
    }

    /// The blend is only meaningful if the two terms are on one scale.
    #[test]
    fn the_blend_moves_the_label_between_the_result_and_the_score() {
        let k = 0.75;
        // A drawn-looking position from a game White went on to win.
        let (outcome, score) = (1.0, 0.0);
        let blend = |lambda: f64| lambda * outcome + (1.0 - lambda) * winning_probability(score, k);

        assert!((blend(1.0) - 1.0).abs() < 1e-9);
        assert!((blend(0.0) - 0.5).abs() < 1e-9);
        assert!((blend(0.5) - 0.75).abs() < 1e-9);
    }
}
