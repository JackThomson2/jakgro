//! Paired self-play match runner for two UCI engines.
//!
//! The runner plays one colour-reversed pair per opening, adjudicates games with
//! the same rules the `cutechess-cli` invocations in `tools/run_match.py` use,
//! and writes a PGN that `tools/analyze_match.py` can validate. It exists so a
//! host without `cutechess-cli` can still produce a paired strength measurement.
//!
//! Statistics are deliberately not computed here: `tools/run_sprt.py` reads the
//! PGN back and owns the pair accounting, so the arbiter and the estimator never
//! share an assumption.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitCode, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::thread;
use std::time::{Duration, Instant};

use cozy_chess::util::{display_san_move, display_uci_move, parse_uci_move};
use cozy_chess::{Board, Color, GameStatus, Piece, Square};

/// Centipawn magnitude assigned to a reported mate score.
const MATE_SCORE_CP: i32 = 30_000;
/// Longest engine reply accepted before a search is treated as a fault.
const DEFAULT_ENGINE_TIMEOUT_MS: u64 = 60_000;
/// Latency allowed on top of a clock before a time forfeit is recorded.
const DEFAULT_TIME_GRACE_MS: u64 = 250;

fn main() -> ExitCode {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments.iter().any(|argument| argument == "--help") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let config = match MatchConfig::parse(&arguments) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("selfplay: {error}");
            return ExitCode::from(2);
        }
    };
    match run_match(&config) {
        Ok(report) => {
            if report.faults.is_empty() {
                ExitCode::SUCCESS
            } else {
                eprintln!("selfplay: {} game(s) ended in a fault", report.faults.len());
                ExitCode::from(3)
            }
        }
        Err(error) => {
            eprintln!("selfplay: {error}");
            ExitCode::from(2)
        }
    }
}

const USAGE: &str = "\
Usage: selfplay --engine PATH [options]

Engines:
  --engine PATH                 candidate UCI executable (required)
  --baseline-engine PATH        baseline executable; defaults to --engine
  --candidate-aggression N      Aggression option for the candidate (default 75)
  --baseline-aggression N       Aggression option for the baseline (default 75)
  --candidate-name NAME         PGN name; defaults from the Aggression value
  --baseline-name NAME          PGN name; defaults from the Aggression value

Match:
  --games N                     even game count (default 96)
  --openings PATH               sequential EPD suite (default tools/data/openings.epd)
  --hash N                      Hash MiB per engine (default 16)
  --threads N                   Threads per engine (default 1)
  --concurrency N               concurrent games (default 8)
  --event NAME                  PGN Event header (default \"Jakgro self-play\")

Limits (choose one; default --nodes 50000):
  --nodes N                     nodes per move
  --movetime-ms N               fixed milliseconds per move
  --time-control BASE+INC       seconds, such as 0.25+0.002

Adjudication:
  --draw-move-number N          earliest draw adjudication move (default 80)
  --draw-move-count N           consecutive quiet full moves (default 10)
  --draw-score N                centipawn window for a draw (default 10)
  --resign-move-count N         consecutive lost full moves (default 4)
  --resign-score N              centipawn resignation threshold (default 800)
  --max-moves N                 draw after this many full moves (default 200)

Output:
  --pgn PATH                    output PGN (required)
  --results-json PATH           optional arbiter-side cross-check summary
  --engine-timeout-ms N         longest accepted engine reply (default 60000)
  --time-grace-ms N             latency allowed before a time forfeit (default 250)
";

/// A per-engine configuration for one side of the match.
#[derive(Clone, Debug, PartialEq, Eq)]
struct EngineConfig {
    path: PathBuf,
    name: String,
    aggression: u8,
}

/// The move limit applied to every search in the match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Limit {
    Nodes(u64),
    MoveTime(Duration),
    Clock { base: Duration, increment: Duration },
}

/// Thresholds for draw, resignation, and move-count adjudication.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Adjudication {
    draw_move_number: u32,
    draw_move_count: usize,
    draw_score: i32,
    resign_move_count: usize,
    resign_score: i32,
    max_moves: u32,
}

/// One validated opening position.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Opening {
    id: String,
    fen: String,
}

/// Every setting the match needs, fully validated.
#[derive(Clone, Debug)]
struct MatchConfig {
    candidate: EngineConfig,
    baseline: EngineConfig,
    games: usize,
    hash_mib: usize,
    threads: usize,
    concurrency: usize,
    limit: Limit,
    adjudication: Adjudication,
    engine_timeout: Duration,
    time_grace: Duration,
    openings: Vec<Opening>,
    pgn: PathBuf,
    results_json: Option<PathBuf>,
    event: String,
}

impl MatchConfig {
    fn parse(arguments: &[String]) -> Result<Self, String> {
        let values = parse_options(arguments)?;
        let mut known = vec![
            "--engine",
            "--baseline-engine",
            "--candidate-aggression",
            "--baseline-aggression",
            "--candidate-name",
            "--baseline-name",
            "--games",
            "--openings",
            "--hash",
            "--threads",
            "--concurrency",
            "--event",
            "--nodes",
            "--movetime-ms",
            "--time-control",
            "--draw-move-number",
            "--draw-move-count",
            "--draw-score",
            "--resign-move-count",
            "--resign-score",
            "--max-moves",
            "--pgn",
            "--results-json",
            "--engine-timeout-ms",
            "--time-grace-ms",
        ];
        known.sort_unstable();
        for key in values.keys() {
            if known.binary_search(&key.as_str()).is_err() {
                return Err(format!("unknown option {key}"));
            }
        }

        let engine = PathBuf::from(require(&values, "--engine")?);
        let baseline_engine = values
            .get("--baseline-engine")
            .map_or_else(|| engine.clone(), PathBuf::from);
        let candidate_aggression = parse_aggression(&values, "--candidate-aggression", 75)?;
        let baseline_aggression = parse_aggression(&values, "--baseline-aggression", 75)?;
        let (candidate_name, baseline_name) = engine_names(
            values.get("--candidate-name").map(String::as_str),
            values.get("--baseline-name").map(String::as_str),
            candidate_aggression,
            baseline_aggression,
        )?;

        let games = parse_number::<usize>(&values, "--games", 96)?;
        if games == 0 || games % 2 != 0 {
            return Err("--games must be a positive even number".to_owned());
        }
        let hash_mib = parse_number::<usize>(&values, "--hash", 16)?;
        if !(1..=1024).contains(&hash_mib) {
            return Err("--hash must be between 1 and 1024 MiB".to_owned());
        }
        let threads = parse_number::<usize>(&values, "--threads", 1)?;
        if !(1..=128).contains(&threads) {
            return Err("--threads must be between 1 and 128".to_owned());
        }
        let concurrency = parse_number::<usize>(&values, "--concurrency", 8)?
            .max(1)
            .min(games / 2);

        let openings_path = values
            .get("--openings")
            .map_or_else(|| PathBuf::from("tools/data/openings.epd"), PathBuf::from);
        let text = fs::read_to_string(&openings_path)
            .map_err(|error| format!("cannot read {}: {error}", openings_path.display()))?;
        let openings = parse_openings(&text)
            .map_err(|error| format!("{}: {error}", openings_path.display()))?;
        if games / 2 > openings.len() {
            return Err(format!(
                "{} games need {} unique openings but the suite has {}",
                games,
                games / 2,
                openings.len()
            ));
        }

        Ok(Self {
            candidate: EngineConfig {
                path: engine,
                name: candidate_name,
                aggression: candidate_aggression,
            },
            baseline: EngineConfig {
                path: baseline_engine,
                name: baseline_name,
                aggression: baseline_aggression,
            },
            games,
            hash_mib,
            threads,
            concurrency,
            limit: parse_limit(&values)?,
            adjudication: parse_adjudication(&values)?,
            engine_timeout: Duration::from_millis(parse_number::<u64>(
                &values,
                "--engine-timeout-ms",
                DEFAULT_ENGINE_TIMEOUT_MS,
            )?),
            time_grace: Duration::from_millis(parse_number::<u64>(
                &values,
                "--time-grace-ms",
                DEFAULT_TIME_GRACE_MS,
            )?),
            openings,
            pgn: PathBuf::from(require(&values, "--pgn")?),
            results_json: values.get("--results-json").map(PathBuf::from),
            event: values
                .get("--event")
                .cloned()
                .unwrap_or_else(|| "Jakgro self-play".to_owned()),
        })
    }
}

fn parse_options(arguments: &[String]) -> Result<BTreeMap<String, String>, String> {
    let mut values = BTreeMap::new();
    let mut index = 0;
    while index < arguments.len() {
        let key = &arguments[index];
        if !key.starts_with("--") {
            return Err(format!("expected an option, found {key}"));
        }
        let value = arguments
            .get(index + 1)
            .ok_or_else(|| format!("{key} needs a value"))?;
        if values.insert(key.clone(), value.clone()).is_some() {
            return Err(format!("{key} was supplied twice"));
        }
        index += 2;
    }
    Ok(values)
}

fn require(values: &BTreeMap<String, String>, key: &str) -> Result<String, String> {
    values
        .get(key)
        .cloned()
        .ok_or_else(|| format!("{key} is required"))
}

fn parse_number<T>(values: &BTreeMap<String, String>, key: &str, default: T) -> Result<T, String>
where
    T: std::str::FromStr,
{
    match values.get(key) {
        None => Ok(default),
        Some(text) => text
            .parse::<T>()
            .map_err(|_| format!("{key} has an invalid value {text}")),
    }
}

fn parse_aggression(
    values: &BTreeMap<String, String>,
    key: &str,
    default: u8,
) -> Result<u8, String> {
    let value = parse_number::<u32>(values, key, u32::from(default))?;
    u8::try_from(value)
        .ok()
        .filter(|aggression| *aggression <= 100)
        .ok_or_else(|| format!("{key} must be between 0 and 100"))
}

fn engine_names(
    candidate: Option<&str>,
    baseline: Option<&str>,
    candidate_aggression: u8,
    baseline_aggression: u8,
) -> Result<(String, String), String> {
    let explicit = candidate.is_some() || baseline.is_some();
    let mut candidate = candidate
        .map(str::to_owned)
        .unwrap_or_else(|| format!("Aggression-{candidate_aggression}"));
    let mut baseline = baseline
        .map(str::to_owned)
        .unwrap_or_else(|| format!("Aggression-{baseline_aggression}"));
    if candidate.trim().is_empty() || baseline.trim().is_empty() {
        return Err("engine names must be non-empty".to_owned());
    }
    if candidate == baseline {
        if explicit {
            return Err("candidate and baseline engine names must differ".to_owned());
        }
        candidate = format!("Candidate-{candidate}");
        baseline = format!("Baseline-{baseline}");
    }
    Ok((candidate, baseline))
}

fn parse_limit(values: &BTreeMap<String, String>) -> Result<Limit, String> {
    let selected = ["--nodes", "--movetime-ms", "--time-control"]
        .into_iter()
        .filter(|key| values.contains_key(*key))
        .collect::<Vec<_>>();
    if selected.len() > 1 {
        return Err(format!("{} are mutually exclusive", selected.join(", ")));
    }
    if let Some(text) = values.get("--time-control") {
        return parse_time_control(text);
    }
    if let Some(text) = values.get("--movetime-ms") {
        let milliseconds = text
            .parse::<u64>()
            .map_err(|_| "--movetime-ms has an invalid value".to_owned())?;
        if milliseconds == 0 {
            return Err("--movetime-ms must be positive".to_owned());
        }
        return Ok(Limit::MoveTime(Duration::from_millis(milliseconds)));
    }
    let nodes = parse_number::<u64>(values, "--nodes", 50_000)?;
    if nodes == 0 {
        return Err("--nodes must be positive".to_owned());
    }
    Ok(Limit::Nodes(nodes))
}

/// Parses a `BASE+INC` control in seconds, matching the Cute Chess spelling.
fn parse_time_control(text: &str) -> Result<Limit, String> {
    let (base_text, increment_text) = match text.split_once('+') {
        Some((base, increment)) => (base, increment),
        None => (text, "0"),
    };
    let base = parse_seconds(base_text).ok_or_else(|| {
        format!("--time-control {text} must look like 0.25+0.002 with seconds only")
    })?;
    let increment = parse_seconds(increment_text)
        .ok_or_else(|| format!("--time-control {text} has an invalid increment"))?;
    if base.is_zero() {
        return Err("--time-control needs a positive base time".to_owned());
    }
    Ok(Limit::Clock { base, increment })
}

fn parse_seconds(text: &str) -> Option<Duration> {
    let seconds = text.trim().parse::<f64>().ok()?;
    if !seconds.is_finite() || !(0.0..=86_400.0).contains(&seconds) {
        return None;
    }
    Some(Duration::from_micros((seconds * 1_000_000.0).round() as u64))
}

fn parse_adjudication(values: &BTreeMap<String, String>) -> Result<Adjudication, String> {
    let adjudication = Adjudication {
        draw_move_number: parse_number(values, "--draw-move-number", 80)?,
        draw_move_count: parse_number(values, "--draw-move-count", 10)?,
        draw_score: parse_number(values, "--draw-score", 10)?,
        resign_move_count: parse_number(values, "--resign-move-count", 4)?,
        resign_score: parse_number(values, "--resign-score", 800)?,
        max_moves: parse_number(values, "--max-moves", 200)?,
    };
    if adjudication.draw_move_count == 0 || adjudication.resign_move_count == 0 {
        return Err("adjudication move counts must be positive".to_owned());
    }
    if adjudication.max_moves == 0 {
        return Err("--max-moves must be positive".to_owned());
    }
    Ok(adjudication)
}

/// Parses a sequential four-field EPD suite with unique ids and positions.
fn parse_openings(text: &str) -> Result<Vec<Opening>, String> {
    let mut openings = Vec::new();
    let mut seen_ids = Vec::new();
    let mut seen_positions = Vec::new();
    for (number, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line_number = number + 1;
        let (position, identifier) = line
            .split_once(" id ")
            .ok_or_else(|| format!("line {line_number}: missing opening id"))?;
        let position = position.trim().trim_end_matches(';').trim();
        if position.split_whitespace().count() != 4 {
            return Err(format!(
                "line {line_number}: expected a four-field EPD position"
            ));
        }
        let identifier = identifier.trim();
        let identifier = identifier
            .strip_prefix('"')
            .and_then(|rest| rest.strip_suffix("\";").or_else(|| rest.strip_suffix('"')))
            .ok_or_else(|| format!("line {line_number}: malformed opening id"))?;
        if identifier.is_empty() {
            return Err(format!("line {line_number}: empty opening id"));
        }
        let fen = format!("{position} 0 1");
        let board = fen
            .parse::<Board>()
            .map_err(|_| format!("line {line_number}: illegal opening position"))?;
        if board.status() != GameStatus::Ongoing {
            return Err(format!("line {line_number}: opening is already decided"));
        }
        if !board.checkers().is_empty() {
            return Err(format!("line {line_number}: opening starts in check"));
        }
        if seen_positions.contains(&position.to_owned()) {
            return Err(format!("line {line_number}: duplicate opening position"));
        }
        if seen_ids.contains(&identifier.to_owned()) {
            return Err(format!("line {line_number}: duplicate opening id"));
        }
        seen_positions.push(position.to_owned());
        seen_ids.push(identifier.to_owned());
        openings.push(Opening {
            id: identifier.to_owned(),
            fen,
        });
    }
    if openings.is_empty() {
        return Err("no opening positions".to_owned());
    }
    Ok(openings)
}

/// The outcome of one completed game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GameResult {
    WhiteWin,
    BlackWin,
    Draw,
}

impl GameResult {
    const fn pgn(self) -> &'static str {
        match self {
            Self::WhiteWin => "1-0",
            Self::BlackWin => "0-1",
            Self::Draw => "1/2-1/2",
        }
    }

    const fn loss_for(color: Color) -> Self {
        match color {
            Color::White => Self::BlackWin,
            Color::Black => Self::WhiteWin,
        }
    }
}

/// A protocol, legality, or timing failure attributed to one engine.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Fault {
    engine: String,
    kind: &'static str,
    detail: String,
}

/// One finished game, ready to be written as PGN.
#[derive(Clone, Debug)]
struct GameRecord {
    round: usize,
    opening: String,
    fen: String,
    white: String,
    black: String,
    result: GameResult,
    termination: &'static str,
    san_moves: Vec<String>,
    fault: Option<Fault>,
}

/// Arbiter-side totals used to cross-check the estimator.
#[derive(Clone, Debug, Default)]
struct MatchReport {
    wins: usize,
    draws: usize,
    losses: usize,
    pair_points: Vec<f64>,
    terminations: BTreeMap<String, usize>,
    faults: Vec<Fault>,
}

fn run_match(config: &MatchConfig) -> Result<MatchReport, String> {
    let pairs = config.games / 2;
    let slots = (0..config.games).map(|_| None).collect::<Vec<_>>();
    let records = Mutex::new(slots);
    let next_pair = AtomicUsize::new(0);
    let completed = AtomicUsize::new(0);

    thread::scope(|scope| {
        for _ in 0..config.concurrency {
            scope.spawn(|| {
                let mut candidate = Engine::new(&config.candidate, config);
                let mut baseline = Engine::new(&config.baseline, config);
                loop {
                    let pair = next_pair.fetch_add(1, Ordering::SeqCst);
                    if pair >= pairs {
                        break;
                    }
                    let opening = &config.openings[pair];
                    for index in 0..2 {
                        let played = if index == 0 {
                            play_game(&mut candidate, &mut baseline, opening, config)
                        } else {
                            play_game(&mut baseline, &mut candidate, opening, config)
                        };
                        let (white, black) = if index == 0 {
                            (&config.candidate.name, &config.baseline.name)
                        } else {
                            (&config.baseline.name, &config.candidate.name)
                        };
                        let record = GameRecord {
                            round: pair + 1,
                            opening: opening.id.clone(),
                            fen: opening.fen.clone(),
                            white: white.clone(),
                            black: black.clone(),
                            result: played.result,
                            termination: played.termination,
                            san_moves: played.san_moves,
                            fault: played.fault,
                        };
                        if let Ok(mut guard) = records.lock() {
                            guard[pair * 2 + index] = Some(record);
                        }
                    }
                    let done = completed.fetch_add(1, Ordering::SeqCst) + 1;
                    eprintln!("selfplay: {done}/{pairs} pairs complete");
                }
            });
        }
    });

    let records = records
        .into_inner()
        .map_err(|_| "match bookkeeping was poisoned".to_owned())?
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| "a game produced no record".to_owned())?;

    write_pgn(&config.pgn, &config.event, &records)
        .map_err(|error| format!("cannot write {}: {error}", config.pgn.display()))?;
    let report = summarize(config, &records);
    if let Some(path) = &config.results_json {
        fs::write(path, results_json(config, &report))
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    }
    eprintln!(
        "selfplay: {} +{} ={} -{} for {}",
        config.games, report.wins, report.draws, report.losses, config.candidate.name,
    );
    Ok(report)
}

fn summarize(config: &MatchConfig, records: &[GameRecord]) -> MatchReport {
    let mut report = MatchReport::default();
    let candidate = &config.candidate.name;
    for chunk in records.chunks(2) {
        let mut points = 0.0;
        for record in chunk {
            let candidate_points = match record.result {
                GameResult::Draw => 0.5,
                GameResult::WhiteWin if &record.white == candidate => 1.0,
                GameResult::BlackWin if &record.black == candidate => 1.0,
                _ => 0.0,
            };
            points += candidate_points;
            if candidate_points == 1.0 {
                report.wins += 1;
            } else if candidate_points == 0.5 {
                report.draws += 1;
            } else {
                report.losses += 1;
            }
            *report
                .terminations
                .entry(record.termination.to_owned())
                .or_default() += 1;
            if let Some(fault) = &record.fault {
                report.faults.push(fault.clone());
            }
        }
        report.pair_points.push(points);
    }
    report
}

/// One game as produced by the arbiter, before PGN identity is attached.
#[derive(Clone, Debug)]
struct PlayedGame {
    result: GameResult,
    termination: &'static str,
    san_moves: Vec<String>,
    fault: Option<Fault>,
}

/// Per-game clock state for a fixed time control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Clocks {
    white: Duration,
    black: Duration,
    increment: Duration,
}

impl Clocks {
    const fn remaining(self, color: Color) -> Duration {
        match color {
            Color::White => self.white,
            Color::Black => self.black,
        }
    }

    fn charge(&mut self, color: Color, elapsed: Duration, grace: Duration) -> bool {
        let remaining = self.remaining(color);
        let Some(next) = charge_clock(remaining, elapsed, self.increment, grace) else {
            return false;
        };
        match color {
            Color::White => self.white = next,
            Color::Black => self.black = next,
        }
        true
    }
}

/// Charges one search against a clock, reporting `None` on a time forfeit.
fn charge_clock(
    remaining: Duration,
    elapsed: Duration,
    increment: Duration,
    grace: Duration,
) -> Option<Duration> {
    if elapsed > remaining.saturating_add(grace) {
        return None;
    }
    Some(
        remaining
            .saturating_sub(elapsed)
            .saturating_add(increment)
            .min(Duration::from_secs(86_400)),
    )
}

/// Tracks reported scores and applies the draw and resignation rules.
#[derive(Clone, Debug, Default)]
struct ScoreHistory {
    /// White-relative score per ply, in playing order.
    plies: Vec<Option<i32>>,
    /// Scores from each side's own perspective, in playing order.
    white: Vec<Option<i32>>,
    black: Vec<Option<i32>>,
}

impl ScoreHistory {
    fn record(&mut self, mover: Color, score: Option<i32>) {
        let white_relative = score.map(|value| match mover {
            Color::White => value,
            Color::Black => -value,
        });
        self.plies.push(white_relative);
        match mover {
            Color::White => self.white.push(score),
            Color::Black => self.black.push(score),
        }
    }

    fn own(&self, color: Color) -> &[Option<i32>] {
        match color {
            Color::White => &self.white,
            Color::Black => &self.black,
        }
    }
}

/// Reports whether the draw rule fires after the move just recorded.
fn draw_adjudicated(history: &ScoreHistory, fullmove: u32, rules: Adjudication) -> bool {
    if fullmove < rules.draw_move_number {
        return false;
    }
    let window = rules.draw_move_count * 2;
    if history.plies.len() < window {
        return false;
    }
    history.plies[history.plies.len() - window..]
        .iter()
        .all(|score| score.is_some_and(|value| value.abs() <= rules.draw_score))
}

/// Reports the side that resigns, if the resignation rule fires.
fn resign_adjudicated(history: &ScoreHistory, rules: Adjudication) -> Option<Color> {
    for color in [Color::White, Color::Black] {
        let own = history.own(color);
        let other = history.own(!color);
        if own.len() < rules.resign_move_count || other.len() < rules.resign_move_count {
            continue;
        }
        let losing = own[own.len() - rules.resign_move_count..]
            .iter()
            .all(|score| score.is_some_and(|value| value <= -rules.resign_score));
        let confirmed = other[other.len() - rules.resign_move_count..]
            .iter()
            .all(|score| score.is_some_and(|value| value >= rules.resign_score));
        if losing && confirmed {
            return Some(color);
        }
    }
    None
}

/// Reports whether neither side retains mating material.
fn is_insufficient_material(board: &Board) -> bool {
    let heavy = board.pieces(Piece::Pawn) | board.pieces(Piece::Rook) | board.pieces(Piece::Queen);
    if !heavy.is_empty() {
        return false;
    }
    let knights = board.pieces(Piece::Knight);
    let bishops = board.pieces(Piece::Bishop);
    let minors = (knights | bishops).into_iter().collect::<Vec<_>>();
    match minors.as_slice() {
        [] | [_] => true,
        [first, second] => {
            knights.is_empty()
                && board.color_on(*first) != board.color_on(*second)
                && square_shade(*first) == square_shade(*second)
        }
        _ => false,
    }
}

const fn square_shade(square: Square) -> u8 {
    (square.file() as u8 + square.rank() as u8) % 2
}

/// Reports whether this position has already occurred twice before.
///
/// `history` holds every position that occurred strictly before `board`, so two
/// earlier occurrences plus the current one is the threefold claim.
fn is_threefold_repetition(history: &[Board], board: &Board) -> bool {
    history
        .iter()
        .filter(|previous| previous.same_position(board))
        .count()
        >= 2
}

/// Reports the terminal outcome of a position, if it has one.
fn terminal_outcome(history: &[Board], board: &Board) -> Option<(GameResult, &'static str)> {
    match board.status() {
        GameStatus::Won => Some((GameResult::loss_for(board.side_to_move()), "checkmate")),
        GameStatus::Drawn => Some((
            GameResult::Draw,
            if board.halfmove_clock() >= 100 {
                "fifty-move rule"
            } else {
                "stalemate"
            },
        )),
        GameStatus::Ongoing => {
            if is_threefold_repetition(history, board) {
                Some((GameResult::Draw, "threefold repetition"))
            } else if is_insufficient_material(board) {
                Some((GameResult::Draw, "insufficient material"))
            } else {
                None
            }
        }
    }
}

fn play_game<'a>(
    white: &mut Engine<'a>,
    black: &mut Engine<'a>,
    opening: &Opening,
    config: &MatchConfig,
) -> PlayedGame {
    let mut board = match opening.fen.parse::<Board>() {
        Ok(board) => board,
        Err(_) => {
            return PlayedGame {
                result: GameResult::Draw,
                termination: "invalid opening",
                san_moves: Vec::new(),
                fault: None,
            };
        }
    };
    let mut history: Vec<Board> = Vec::new();
    let mut uci_moves: Vec<String> = Vec::new();
    let mut san_moves: Vec<String> = Vec::new();
    let mut scores = ScoreHistory::default();
    let mut clocks = match config.limit {
        Limit::Clock { base, increment } => Clocks {
            white: base,
            black: base,
            increment,
        },
        _ => Clocks {
            white: Duration::ZERO,
            black: Duration::ZERO,
            increment: Duration::ZERO,
        },
    };
    for engine in [&mut *white, &mut *black] {
        engine.start_game();
    }

    loop {
        if let Some((result, termination)) = terminal_outcome(&history, &board) {
            return PlayedGame {
                result,
                termination,
                san_moves,
                fault: None,
            };
        }
        if u32::from(board.fullmove_number()) > config.adjudication.max_moves {
            return PlayedGame {
                result: GameResult::Draw,
                termination: "adjudication: move limit",
                san_moves,
                fault: None,
            };
        }

        let mover = board.side_to_move();
        let engine = match mover {
            Color::White => &mut *white,
            Color::Black => &mut *black,
        };
        let position = position_command(&opening.fen, &uci_moves);
        let go = go_command(config.limit, clocks);
        let allowance = match config.limit {
            Limit::Clock { .. } => clocks
                .remaining(mover)
                .saturating_add(config.time_grace)
                .saturating_add(config.engine_timeout),
            Limit::MoveTime(move_time) => move_time.saturating_add(config.engine_timeout),
            Limit::Nodes(_) => config.engine_timeout,
        };
        let outcome = match engine.search(&position, &go, allowance) {
            Ok(outcome) => outcome,
            Err(fault) => {
                engine.restart();
                return PlayedGame {
                    result: GameResult::loss_for(mover),
                    termination: fault.kind,
                    san_moves,
                    fault: Some(fault),
                };
            }
        };
        let Ok(chess_move) = parse_uci_move(&board, &outcome.best_move) else {
            let fault = engine.fault("illegal move", &outcome.best_move);
            engine.restart();
            return PlayedGame {
                result: GameResult::loss_for(mover),
                termination: fault.kind,
                san_moves,
                fault: Some(fault),
            };
        };
        if !board.is_legal(chess_move) {
            let fault = engine.fault("illegal move", &outcome.best_move);
            engine.restart();
            return PlayedGame {
                result: GameResult::loss_for(mover),
                termination: fault.kind,
                san_moves,
                fault: Some(fault),
            };
        }
        if matches!(config.limit, Limit::Clock { .. })
            && !clocks.charge(mover, outcome.elapsed, config.time_grace)
        {
            let fault = engine.fault(
                "time forfeit",
                &format!("used {} ms", outcome.elapsed.as_millis()),
            );
            return PlayedGame {
                result: GameResult::loss_for(mover),
                termination: fault.kind,
                san_moves,
                fault: Some(fault),
            };
        }

        scores.record(mover, outcome.score);
        san_moves.push(display_san_move(&board, chess_move).to_string());
        uci_moves.push(display_uci_move(&board, chess_move).to_string());
        let fullmove = u32::from(board.fullmove_number());
        history.push(board.clone());
        board.play_unchecked(chess_move);

        if let Some(resigning) = resign_adjudicated(&scores, config.adjudication) {
            return PlayedGame {
                result: GameResult::loss_for(resigning),
                termination: "adjudication: resignation",
                san_moves,
                fault: None,
            };
        }
        if draw_adjudicated(&scores, fullmove, config.adjudication) {
            return PlayedGame {
                result: GameResult::Draw,
                termination: "adjudication: drawn score",
                san_moves,
                fault: None,
            };
        }
    }
}

fn position_command(fen: &str, moves: &[String]) -> String {
    let mut command = format!("position fen {fen}");
    if !moves.is_empty() {
        command.push_str(" moves ");
        command.push_str(&moves.join(" "));
    }
    command
}

/// Renders the `go` command for one search.
///
/// A clocked search reports both clocks and both increments; the engine takes
/// the side to move from the position, so the mover is not encoded here. Clocks
/// are floored at one millisecond because a zero clock is indistinguishable from
/// an absent field over the protocol.
fn go_command(limit: Limit, clocks: Clocks) -> String {
    match limit {
        Limit::Nodes(nodes) => format!("go nodes {nodes}"),
        Limit::MoveTime(move_time) => format!("go movetime {}", move_time.as_millis().max(1)),
        Limit::Clock { .. } => {
            let increment = clocks.increment.as_millis();
            format!(
                "go wtime {} btime {} winc {increment} binc {increment}",
                clocks.white.as_millis().max(1),
                clocks.black.as_millis().max(1),
            )
        }
    }
}

/// One completed engine search.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SearchOutcome {
    best_move: String,
    score: Option<i32>,
    elapsed: Duration,
}

/// Extracts a centipawn score from an `info` line, if it reports one.
fn parse_info_score(line: &str) -> Option<i32> {
    let tokens = line.split_whitespace().collect::<Vec<_>>();
    let index = tokens.iter().position(|token| *token == "score")?;
    match (tokens.get(index + 1), tokens.get(index + 2)) {
        (Some(&"cp"), Some(value)) => value.parse::<i32>().ok(),
        (Some(&"mate"), Some(value)) => value.parse::<i32>().ok().map(|moves| {
            let magnitude = MATE_SCORE_CP - moves.abs().saturating_mul(2);
            if moves < 0 { -magnitude } else { magnitude }
        }),
        _ => None,
    }
}

/// Extracts the move from a `bestmove` line.
fn parse_best_move(line: &str) -> Option<String> {
    let mut tokens = line.split_whitespace();
    if tokens.next() != Some("bestmove") {
        return None;
    }
    tokens
        .next()
        .filter(|token| *token != "(none)" && *token != "0000")
        .map(str::to_owned)
}

/// A running engine process behind its UCI pipes.
struct Process {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
}

/// One side of the match, restarted on demand after a fault.
struct Engine<'a> {
    config: &'a EngineConfig,
    hash_mib: usize,
    threads: usize,
    handshake_timeout: Duration,
    process: Option<Process>,
}

impl<'a> Engine<'a> {
    fn new(config: &'a EngineConfig, match_config: &MatchConfig) -> Self {
        Self {
            config,
            hash_mib: match_config.hash_mib,
            threads: match_config.threads,
            handshake_timeout: match_config.engine_timeout,
            process: None,
        }
    }

    fn fault(&self, kind: &'static str, detail: &str) -> Fault {
        Fault {
            engine: self.config.name.clone(),
            kind,
            detail: detail.to_owned(),
        }
    }

    fn restart(&mut self) {
        if let Some(mut process) = self.process.take() {
            let _ = writeln!(process.stdin, "quit");
            let _ = process.stdin.flush();
            let _ = process.child.kill();
            let _ = process.child.wait();
        }
    }

    fn start_game(&mut self) {
        if let Some(process) = self.process.as_mut() {
            if writeln!(process.stdin, "ucinewgame")
                .and_then(|()| process.stdin.flush())
                .is_err()
            {
                self.restart();
            }
        }
    }

    fn ensure_started(&mut self) -> Result<(), Fault> {
        if self.process.is_some() {
            return Ok(());
        }
        let mut child = Command::new(&self.config.path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| self.fault("disconnection", &error.to_string()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| self.fault("disconnection", "engine has no stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| self.fault("disconnection", "engine has no stdout"))?;
        let (sender, lines) = channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        self.process = Some(Process {
            child,
            stdin,
            lines,
        });
        self.handshake()
    }

    fn handshake(&mut self) -> Result<(), Fault> {
        let aggression = self.config.aggression;
        let hash_mib = self.hash_mib;
        let threads = self.threads;
        self.send("uci")?;
        self.expect("uciok", self.handshake_timeout)?;
        self.send(&format!("setoption name Hash value {hash_mib}"))?;
        self.send(&format!("setoption name Threads value {threads}"))?;
        self.send(&format!("setoption name Aggression value {aggression}"))?;
        self.send("isready")?;
        self.expect("readyok", self.handshake_timeout)?;
        Ok(())
    }

    fn send(&mut self, command: &str) -> Result<(), Fault> {
        let process = self.process.as_mut().ok_or_else(|| Fault {
            engine: self.config.name.clone(),
            kind: "disconnection",
            detail: "engine is not running".to_owned(),
        })?;
        writeln!(process.stdin, "{command}")
            .and_then(|()| process.stdin.flush())
            .map_err(|error| Fault {
                engine: self.config.name.clone(),
                kind: "disconnection",
                detail: error.to_string(),
            })
    }

    fn read_line(&mut self, deadline: Instant) -> Result<String, Fault> {
        let name = self.config.name.clone();
        let process = self.process.as_mut().ok_or_else(|| Fault {
            engine: name.clone(),
            kind: "disconnection",
            detail: "engine is not running".to_owned(),
        })?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        match process.lines.recv_timeout(remaining) {
            Ok(line) => Ok(line),
            Err(RecvTimeoutError::Timeout) => Err(Fault {
                engine: name,
                kind: "engine timeout",
                detail: "no reply before the deadline".to_owned(),
            }),
            Err(RecvTimeoutError::Disconnected) => Err(Fault {
                engine: name,
                kind: "disconnection",
                detail: "engine closed its output".to_owned(),
            }),
        }
    }

    fn expect(&mut self, token: &str, allowance: Duration) -> Result<(), Fault> {
        let deadline = Instant::now() + allowance;
        loop {
            let line = self.read_line(deadline)?;
            if line.trim() == token {
                return Ok(());
            }
        }
    }

    fn search(
        &mut self,
        position: &str,
        go: &str,
        allowance: Duration,
    ) -> Result<SearchOutcome, Fault> {
        self.ensure_started()?;
        self.send(position)?;
        let started = Instant::now();
        self.send(go)?;
        let deadline = started + allowance;
        let mut score = None;
        loop {
            let line = self.read_line(deadline)?;
            let trimmed = line.trim();
            if trimmed.starts_with("info ") {
                if let Some(value) = parse_info_score(trimmed) {
                    score = Some(value);
                }
                continue;
            }
            if trimmed.starts_with("bestmove") {
                let elapsed = started.elapsed();
                let best_move = parse_best_move(trimmed).ok_or_else(|| Fault {
                    engine: self.config.name.clone(),
                    kind: "illegal move",
                    detail: trimmed.to_owned(),
                })?;
                return Ok(SearchOutcome {
                    best_move,
                    score,
                    elapsed,
                });
            }
        }
    }
}

impl Drop for Engine<'_> {
    fn drop(&mut self) {
        self.restart();
    }
}

fn write_pgn(path: &Path, event: &str, records: &[GameRecord]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let mut text = String::new();
    for record in records {
        text.push_str(&render_game(event, record));
    }
    fs::write(path, text)
}

fn render_game(event: &str, record: &GameRecord) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "[Event \"{}\"]", escape_header(event));
    let _ = writeln!(text, "[Site \"selfplay\"]");
    let _ = writeln!(text, "[Date \"????.??.??\"]");
    let _ = writeln!(text, "[Round \"{}\"]", record.round);
    let _ = writeln!(text, "[White \"{}\"]", escape_header(&record.white));
    let _ = writeln!(text, "[Black \"{}\"]", escape_header(&record.black));
    let _ = writeln!(text, "[Result \"{}\"]", record.result.pgn());
    let _ = writeln!(text, "[FEN \"{}\"]", escape_header(&record.fen));
    let _ = writeln!(text, "[SetUp \"1\"]");
    let _ = writeln!(text, "[PlyCount \"{}\"]", record.san_moves.len());
    let _ = writeln!(
        text,
        "[Termination \"{}\"]",
        escape_header(record.termination)
    );
    let _ = writeln!(text, "[Opening \"{}\"]", escape_header(&record.opening));
    text.push('\n');
    text.push_str(&render_movetext(record));
    text.push('\n');
    text
}

fn escape_header(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Renders numbered movetext ending in the result token, wrapped for PGN.
fn render_movetext(record: &GameRecord) -> String {
    let black_first = record
        .fen
        .split_whitespace()
        .nth(1)
        .is_some_and(|field| field == "b");
    let first_number = record
        .fen
        .split_whitespace()
        .nth(5)
        .and_then(|field| field.parse::<u32>().ok())
        .unwrap_or(1);
    let mut tokens = Vec::new();
    let mut number = first_number;
    let mut black_to_move = black_first;
    for san in &record.san_moves {
        if black_to_move {
            if tokens.is_empty() {
                tokens.push(format!("{number}..."));
            }
            tokens.push(san.clone());
            number += 1;
        } else {
            tokens.push(format!("{number}."));
            tokens.push(san.clone());
        }
        black_to_move = !black_to_move;
    }
    tokens.push(record.result.pgn().to_owned());

    let mut lines = Vec::new();
    let mut line = String::new();
    for token in tokens {
        if !line.is_empty() && line.len() + 1 + token.len() > 79 {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(&token);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

fn results_json(config: &MatchConfig, report: &MatchReport) -> String {
    let mut text = String::from("{\n  \"schema_version\": 1,\n");
    let _ = writeln!(text, "  \"games\": {},", config.games);
    let _ = writeln!(text, "  \"pairs\": {},", report.pair_points.len());
    let _ = writeln!(
        text,
        "  \"candidate\": \"{}\",",
        escape_header(&config.candidate.name)
    );
    let _ = writeln!(
        text,
        "  \"baseline\": \"{}\",",
        escape_header(&config.baseline.name)
    );
    let _ = writeln!(text, "  \"wins\": {},", report.wins);
    let _ = writeln!(text, "  \"draws\": {},", report.draws);
    let _ = writeln!(text, "  \"losses\": {},", report.losses);
    let points = report
        .pair_points
        .iter()
        .map(|points| format!("{points:.1}"))
        .collect::<Vec<_>>()
        .join(", ");
    let _ = writeln!(text, "  \"pair_points\": [{points}],");
    let terminations = report
        .terminations
        .iter()
        .map(|(name, count)| format!("\n    \"{}\": {count}", escape_header(name)))
        .collect::<Vec<_>>()
        .join(",");
    let _ = writeln!(text, "  \"terminations\": {{{terminations}\n  }},");
    let faults = report
        .faults
        .iter()
        .map(|fault| {
            format!(
                "\n    {{\"engine\": \"{}\", \"kind\": \"{}\", \"detail\": \"{}\"}}",
                escape_header(&fault.engine),
                escape_header(fault.kind),
                escape_header(&fault.detail),
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let _ = writeln!(text, "  \"faults\": [{faults}\n  ]");
    text.push_str("}\n");
    text
}

#[cfg(test)]
mod tests {
    use super::{
        Adjudication, Clocks, GameRecord, GameResult, Limit, Opening, ScoreHistory, charge_clock,
        draw_adjudicated, engine_names, go_command, is_insufficient_material,
        is_threefold_repetition, parse_best_move, parse_info_score, parse_openings,
        parse_time_control, position_command, render_movetext, resign_adjudicated,
        terminal_outcome,
    };
    use cozy_chess::util::parse_uci_move;
    use cozy_chess::{Board, Color};
    use std::time::Duration;

    const RULES: Adjudication = Adjudication {
        draw_move_number: 3,
        draw_move_count: 2,
        draw_score: 10,
        resign_move_count: 2,
        resign_score: 800,
        max_moves: 200,
    };

    fn history(scores: &[(Color, Option<i32>)]) -> ScoreHistory {
        let mut history = ScoreHistory::default();
        for (mover, score) in scores {
            history.record(*mover, *score);
        }
        history
    }

    #[test]
    fn openings_are_parsed_and_validated() {
        let suite = concat!(
            "# comment\n",
            "r1bqk1nr/pppp1ppp/2n5/2b1p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - id \"italian\";\n",
            "rnbqkb1r/ppp2ppp/5n2/3pp3/4PP2/2N5/PPPP2PP/R1BQKBNR w KQkq d6 id \"vienna\";\n",
        );

        let openings = parse_openings(suite).unwrap();

        assert_eq!(openings.len(), 2);
        assert_eq!(
            openings[0],
            Opening {
                id: "italian".to_owned(),
                fen: "r1bqk1nr/pppp1ppp/2n5/2b1p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 0 1"
                    .to_owned(),
            }
        );
        assert!(openings[1].fen.ends_with(" d6 0 1"));
    }

    #[test]
    fn the_repository_opening_suite_is_accepted() {
        let text = std::fs::read_to_string("tools/data/openings.epd").unwrap();

        assert_eq!(parse_openings(&text).unwrap().len(), 48);
    }

    #[test]
    fn malformed_openings_are_rejected() {
        let missing_id = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq -\n";
        let duplicate = concat!(
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - id \"one\";\n",
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - id \"two\";\n",
        );
        let in_check = "rnb1kbnr/pppp1ppp/8/8/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - id \"check\";\n";

        assert!(parse_openings(missing_id).is_err());
        assert!(parse_openings(duplicate).is_err());
        assert!(parse_openings(in_check).is_err());
        assert!(parse_openings("# only comments\n").is_err());
    }

    #[test]
    fn engine_names_are_disambiguated_only_when_derived() {
        assert_eq!(
            engine_names(None, None, 75, 0).unwrap(),
            ("Aggression-75".to_owned(), "Aggression-0".to_owned())
        );
        assert_eq!(
            engine_names(None, None, 75, 75).unwrap(),
            (
                "Candidate-Aggression-75".to_owned(),
                "Baseline-Aggression-75".to_owned()
            )
        );
        assert!(engine_names(Some("same"), Some("same"), 75, 0).is_err());
        assert!(engine_names(Some(" "), None, 75, 0).is_err());
    }

    #[test]
    fn time_controls_are_parsed_in_seconds() {
        assert_eq!(
            parse_time_control("0.25+0.002").unwrap(),
            Limit::Clock {
                base: Duration::from_millis(250),
                increment: Duration::from_micros(2_000),
            }
        );
        assert_eq!(
            parse_time_control("10").unwrap(),
            Limit::Clock {
                base: Duration::from_secs(10),
                increment: Duration::ZERO,
            }
        );
        assert!(parse_time_control("0+1").is_err());
        assert!(parse_time_control("abc").is_err());
    }

    #[test]
    fn info_and_bestmove_lines_are_parsed() {
        assert_eq!(
            parse_info_score("info depth 5 score cp -37 nodes 10 pv e2e4"),
            Some(-37)
        );
        assert_eq!(
            parse_info_score("info depth 5 score mate 3 nodes 10"),
            Some(super::MATE_SCORE_CP - 6)
        );
        assert_eq!(
            parse_info_score("info depth 5 score mate -2 nodes 10"),
            Some(-(super::MATE_SCORE_CP - 4))
        );
        assert_eq!(parse_info_score("info depth 5 nodes 10"), None);
        assert_eq!(
            parse_best_move("bestmove e2e4 ponder e7e5"),
            Some("e2e4".to_owned())
        );
        assert_eq!(parse_best_move("bestmove (none)"), None);
        assert_eq!(parse_best_move("info string bestmove"), None);
    }

    #[test]
    fn commands_carry_the_position_and_limit() {
        assert_eq!(
            position_command("8/8/8/8/8/8/8/K1k5 w - - 0 1", &[]),
            "position fen 8/8/8/8/8/8/8/K1k5 w - - 0 1"
        );
        assert_eq!(
            position_command("8/8/8/8/8/8/8/K1k5 w - - 0 1", &["a1a2".to_owned()]),
            "position fen 8/8/8/8/8/8/8/K1k5 w - - 0 1 moves a1a2"
        );
        assert_eq!(
            go_command(Limit::Nodes(1_000), ZERO_CLOCKS),
            "go nodes 1000"
        );
        assert_eq!(
            go_command(Limit::MoveTime(Duration::from_millis(40)), ZERO_CLOCKS),
            "go movetime 40"
        );
        assert_eq!(
            go_command(
                Limit::Clock {
                    base: Duration::from_millis(250),
                    increment: Duration::from_millis(2)
                },
                Clocks {
                    white: Duration::from_millis(250),
                    black: Duration::from_millis(200),
                    increment: Duration::from_millis(2),
                }
            ),
            "go wtime 250 btime 200 winc 2 binc 2"
        );
    }

    const ZERO_CLOCKS: Clocks = Clocks {
        white: Duration::ZERO,
        black: Duration::ZERO,
        increment: Duration::ZERO,
    };

    #[test]
    fn clocks_are_charged_and_forfeited() {
        assert_eq!(
            charge_clock(
                Duration::from_millis(100),
                Duration::from_millis(40),
                Duration::from_millis(5),
                Duration::from_millis(10),
            ),
            Some(Duration::from_millis(65))
        );
        assert_eq!(
            charge_clock(
                Duration::from_millis(100),
                Duration::from_millis(105),
                Duration::ZERO,
                Duration::from_millis(10),
            ),
            Some(Duration::ZERO)
        );
        assert_eq!(
            charge_clock(
                Duration::from_millis(100),
                Duration::from_millis(200),
                Duration::ZERO,
                Duration::from_millis(10),
            ),
            None
        );
    }

    #[test]
    fn the_draw_rule_needs_a_quiet_window_after_its_move_number() {
        let quiet = history(&[
            (Color::White, Some(4)),
            (Color::Black, Some(-3)),
            (Color::White, Some(2)),
            (Color::Black, Some(1)),
        ]);

        assert!(!draw_adjudicated(&quiet, 2, RULES));
        assert!(draw_adjudicated(&quiet, 3, RULES));

        let loud = history(&[
            (Color::White, Some(4)),
            (Color::Black, Some(-3)),
            (Color::White, Some(90)),
            (Color::Black, Some(1)),
        ]);
        assert!(!draw_adjudicated(&loud, 9, RULES));

        let unscored = history(&[
            (Color::White, Some(4)),
            (Color::Black, Some(-3)),
            (Color::White, None),
            (Color::Black, Some(1)),
        ]);
        assert!(!draw_adjudicated(&unscored, 9, RULES));
    }

    #[test]
    fn resignation_requires_agreement_from_both_sides() {
        let agreed = history(&[
            (Color::White, Some(900)),
            (Color::Black, Some(-900)),
            (Color::White, Some(1_000)),
            (Color::Black, Some(-1_000)),
        ]);
        assert_eq!(resign_adjudicated(&agreed, RULES), Some(Color::Black));

        let one_sided = history(&[
            (Color::White, Some(10)),
            (Color::Black, Some(-900)),
            (Color::White, Some(20)),
            (Color::Black, Some(-1_000)),
        ]);
        assert_eq!(resign_adjudicated(&one_sided, RULES), None);

        let short = history(&[(Color::White, Some(900)), (Color::Black, Some(-900))]);
        assert_eq!(resign_adjudicated(&short, RULES), None);
    }

    #[test]
    fn terminal_positions_are_classified() {
        let mate = "7k/6Q1/6K1/8/8/8/8/8 b - - 0 1".parse::<Board>().unwrap();
        let stalemate = "7k/5Q2/6K1/8/8/8/8/8 b - - 0 1".parse::<Board>().unwrap();
        let bare = "7k/8/8/8/8/8/8/K7 w - - 0 1".parse::<Board>().unwrap();
        let fifty = "7k/8/8/8/8/8/4R3/4K3 w - - 100 60"
            .parse::<Board>()
            .unwrap();

        assert_eq!(
            terminal_outcome(&[], &mate),
            Some((GameResult::WhiteWin, "checkmate"))
        );
        assert_eq!(
            terminal_outcome(&[], &stalemate),
            Some((GameResult::Draw, "stalemate"))
        );
        assert_eq!(
            terminal_outcome(&[], &bare),
            Some((GameResult::Draw, "insufficient material"))
        );
        assert_eq!(
            terminal_outcome(&[], &fifty),
            Some((GameResult::Draw, "fifty-move rule"))
        );

        let start = Board::default();
        assert_eq!(terminal_outcome(&[], &start), None);
        assert_eq!(terminal_outcome(std::slice::from_ref(&start), &start), None);
    }

    #[test]
    fn insufficient_material_covers_only_drawn_configurations() {
        for fen in [
            "7k/8/8/8/8/8/8/K7 w - - 0 1",
            "7k/8/8/8/8/8/8/KN6 w - - 0 1",
            "7k/8/8/8/8/8/8/KB6 w - - 0 1",
            "6bk/8/8/8/8/8/8/KB6 w - - 0 1",
        ] {
            let board = fen.parse::<Board>().unwrap();
            assert!(is_insufficient_material(&board), "{fen} should be drawn");
        }
        for fen in [
            "7k/8/8/8/8/8/8/KR6 w - - 0 1",
            "7k/8/8/8/8/8/P7/K7 w - - 0 1",
            "7k/8/8/8/8/8/8/KNN5 w - - 0 1",
            "5bk1/8/8/8/8/8/8/KB6 w - - 0 1",
        ] {
            let board = fen.parse::<Board>().unwrap();
            assert!(!is_insufficient_material(&board), "{fen} should play on");
        }
    }

    #[test]
    fn threefold_repetition_needs_two_earlier_occurrences() {
        let mut board = Board::default();
        let mut history = vec![board.clone()];
        for text in ["g1f3", "g8f6", "f3g1", "f6g8", "g1f3", "g8f6", "f3g1"] {
            let chess_move = parse_uci_move(&board, text).unwrap();
            board.play_unchecked(chess_move);
            assert!(!is_threefold_repetition(&history, &board));
            history.push(board.clone());
        }
        let chess_move = parse_uci_move(&board, "f6g8").unwrap();
        board.play_unchecked(chess_move);

        assert!(is_threefold_repetition(&history, &board));
    }

    #[test]
    fn movetext_numbers_from_the_opening_side_and_ends_with_the_result() {
        let white_first = GameRecord {
            round: 1,
            opening: "start".to_owned(),
            fen: "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1".to_owned(),
            white: "A".to_owned(),
            black: "B".to_owned(),
            result: GameResult::WhiteWin,
            termination: "checkmate",
            san_moves: vec!["e4".to_owned(), "e5".to_owned(), "Nf3".to_owned()],
            fault: None,
        };
        let black_first = GameRecord {
            fen: "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 3".to_owned(),
            result: GameResult::Draw,
            san_moves: vec!["e5".to_owned(), "Nf3".to_owned()],
            ..white_first.clone()
        };

        assert_eq!(render_movetext(&white_first), "1. e4 e5 2. Nf3 1-0\n");
        assert_eq!(render_movetext(&black_first), "3... e5 4. Nf3 1/2-1/2\n");
    }
}
