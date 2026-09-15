//! Streaming dataset preparation: features, deduplication, split hygiene,
//! statistics and hash-bound manifests, in one pass per split.
//!
//! Inputs are plain `FEN;white-outcome[;white-score-cp]` lines. Each kept row is
//! written with its canonical key, side to move, labels and both perspectives'
//! sorted feature indices. Duplicates are rejected unless `--deduplicate` keeps
//! the first occurrence, and a development row that repeats any training
//! position (by canonical key, or by feature identity including colour/rank
//! mirrors and turn changes) is rejected unless `--drop-development-overlap`
//! removes it. Neither rule is a game-independence certificate.

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use cozy_chess::Color;
use jakgro::engine::nnue::{
    ACTIVATION_MAX, HIDDEN_SIZE, INPUT_FEATURES, KING_BUCKETS, OUTPUT_SCALE, PIECE_PLANES,
    active_features,
};

use crate::{HEADER, canonical, comma, parse_sample, sha256};

#[derive(Debug)]
pub struct Options {
    pub training: PathBuf,
    pub development: PathBuf,
    pub output: PathBuf,
    pub deduplicate: bool,
    pub drop_overlap: bool,
}

impl Options {
    pub fn parse(arguments: &[String]) -> Result<Self, String> {
        let mut values = BTreeMap::new();
        let mut flags = HashSet::new();
        let mut index = 0;
        while index < arguments.len() {
            let key = arguments[index].as_str();
            match key {
                "--deduplicate" | "--drop-development-overlap" => {
                    if !flags.insert(key) {
                        return Err(format!("repeated option {key}"));
                    }
                    index += 1;
                }
                "--training" | "--development" | "--output-dir" => {
                    let value = arguments
                        .get(index + 1)
                        .ok_or_else(|| format!("{key} needs a value"))?;
                    if values.insert(key, PathBuf::from(value)).is_some() {
                        return Err(format!("repeated option {key}"));
                    }
                    index += 2;
                }
                _ => return Err(format!("unknown option {key}")),
            }
        }
        let path = |key: &str| {
            values
                .get(key)
                .cloned()
                .ok_or_else(|| format!("prepare needs {key}"))
        };
        let options = Self {
            training: path("--training")?,
            development: path("--development")?,
            output: path("--output-dir")?,
            deduplicate: flags.contains("--deduplicate"),
            drop_overlap: flags.contains("--drop-development-overlap"),
        };
        if options.training == options.development {
            return Err("training and development are the same file".to_owned());
        }
        Ok(options)
    }
}

/// Exact identities of every kept row in a split.
#[derive(Default)]
struct Keys {
    canonical: HashSet<String>,
    /// Both perspectives' features, smaller vector first, joined by a sentinel.
    identity: HashSet<Vec<u16>>,
}

fn identity(white: &[u16], black: &[u16]) -> Vec<u16> {
    let (first, second) = if white <= black {
        (white, black)
    } else {
        (black, white)
    };
    let mut key = Vec::with_capacity(first.len() + second.len() + 1);
    key.extend_from_slice(first);
    key.push(u16::MAX);
    key.extend_from_slice(second);
    key
}

#[derive(Default)]
pub struct SplitStats {
    pub raw_rows: usize,
    pub rows: usize,
    pub white_to_move: usize,
    /// Counts of White outcomes 0, 1/2 and 1.
    pub white_outcomes: [usize; 3],
    pub teacher_rows: usize,
    /// Rows per king bucket for the White and Black perspectives.
    pub king_bucket_rows: [[usize; KING_BUCKETS]; 2],
    pub feature_support: Vec<u32>,
    pub dropped: BTreeMap<&'static str, usize>,
}

impl SplitStats {
    fn new() -> Self {
        Self {
            feature_support: vec![0; INPUT_FEATURES],
            ..Self::default()
        }
    }

    fn json(&self, with_support: bool) -> String {
        let mut text = format!(
            "{{\"raw_rows\": {}, \"rows\": {}, \"white_to_move\": {}, \"white_outcomes\": {{\"0.0\": {}, \"0.5\": {}, \"1.0\": {}}}, \"teacher_rows\": {}, \"king_bucket_rows\": [[{}], [{}]], \"dropped\": {{",
            self.raw_rows,
            self.rows,
            self.white_to_move,
            self.white_outcomes[0],
            self.white_outcomes[1],
            self.white_outcomes[2],
            self.teacher_rows,
            comma(self.king_bucket_rows[0]),
            comma(self.king_bucket_rows[1]),
        );
        for (index, (reason, count)) in self.dropped.iter().enumerate() {
            if index > 0 {
                text.push_str(", ");
            }
            let _ = write!(text, "\"{reason}\": {count}");
        }
        text.push('}');
        if with_support {
            let _ = write!(
                text,
                ", \"feature_support\": [{}]",
                comma(&self.feature_support)
            );
        }
        text.push('}');
        text
    }
}

/// One parsed input line, ready for the sequential deduplication pass.
struct Parsed {
    key: String,
    identity: Vec<u16>,
    white_to_move: bool,
    outcome: f64,
    teacher: bool,
    white: Vec<u16>,
    black: Vec<u16>,
    row: String,
}

/// Lines per parallel batch; bounds the memory held between passes.
const PARSE_BATCH_LINES: usize = 1 << 20;

fn parse_line(line: &str) -> Result<Parsed, String> {
    let sample = parse_sample(line)?;
    let white = active_features(&sample.board, Color::White);
    let black = active_features(&sample.board, Color::Black);
    let key = canonical(&sample.board);
    let teacher = sample
        .teacher
        .map_or_else(|| "-".to_owned(), |score| score.to_string());
    let row = format!(
        "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
        sample.board,
        key,
        u8::from(sample.board.side_to_move() == Color::Black),
        sample.outcome,
        teacher,
        comma(&white),
        comma(&black)
    );
    Ok(Parsed {
        identity: identity(&white, &black),
        key,
        white_to_move: sample.board.side_to_move() == Color::White,
        outcome: sample.outcome,
        teacher: sample.teacher.is_some(),
        white,
        black,
        row,
    })
}

/// Parses a batch of numbered lines on every available thread, in order.
fn parse_batch(batch: &[(usize, &str)]) -> Result<Vec<Parsed>, String> {
    let threads = std::thread::available_parallelism().map_or(1, |count| count.get());
    let chunk = batch.len().div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        let handles = batch
            .chunks(chunk)
            .map(|lines| {
                scope.spawn(move || {
                    lines
                        .iter()
                        .map(|&(number, line)| {
                            parse_line(line).map_err(|error| format!("line {number}: {error}"))
                        })
                        .collect::<Result<Vec<_>, _>>()
                })
            })
            .collect::<Vec<_>>();
        let mut parsed = Vec::with_capacity(batch.len());
        for handle in handles {
            parsed.extend(handle.join().expect("parser thread panicked")?);
        }
        Ok(parsed)
    })
}

fn process_split(
    source: &Path,
    target: &Path,
    keys: &mut Keys,
    training: Option<&Keys>,
    deduplicate: bool,
    drop_overlap: bool,
) -> Result<SplitStats, String> {
    let bytes = fs::read(source).map_err(|error| format!("{}: {error}", source.display()))?;
    let text = std::str::from_utf8(&bytes).map_err(|error| {
        format!(
            "{}: invalid UTF-8 at byte {}",
            source.display(),
            error.valid_up_to()
        )
    })?;
    let mut writer = BufWriter::new(
        File::create(target).map_err(|error| format!("{}: {error}", target.display()))?,
    );
    writer
        .write_all(HEADER.as_bytes())
        .map_err(|error| error.to_string())?;
    let mut stats = SplitStats::new();
    let mut batch: Vec<(usize, &str)> = Vec::with_capacity(PARSE_BATCH_LINES);
    let mut lines = text.lines().enumerate().peekable();
    while lines.peek().is_some() {
        batch.clear();
        for (index, line) in lines.by_ref() {
            let number = index + 1;
            if line.len() as u64 > crate::MAX_LINE_BYTES {
                return Err(format!(
                    "line {number}: record exceeds {} bytes",
                    crate::MAX_LINE_BYTES
                ));
            }
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            batch.push((number, line));
            if batch.len() == PARSE_BATCH_LINES {
                break;
            }
        }
        for parsed in parse_batch(&batch)? {
            stats.raw_rows += 1;
            let reason = if keys.canonical.contains(&parsed.key) {
                Some(("canonical_duplicate", deduplicate))
            } else if keys.identity.contains(&parsed.identity) {
                Some(("feature_duplicate", deduplicate))
            } else if training.is_some_and(|training| {
                training.canonical.contains(&parsed.key)
                    || training.identity.contains(&parsed.identity)
            }) {
                Some(("training_overlap", drop_overlap))
            } else {
                None
            };
            if let Some((reason, allowed)) = reason {
                if !allowed {
                    return Err(match reason {
                        "training_overlap" => "training/development overlap (including omitted metadata, turn changes and colour mirrors); use --drop-development-overlap to remove such rows".to_owned(),
                        _ => format!("{reason}: use --deduplicate to explicitly keep only the first occurrence"),
                    });
                }
                *stats.dropped.entry(reason).or_default() += 1;
                continue;
            }
            stats.rows += 1;
            stats.white_to_move += usize::from(parsed.white_to_move);
            stats.white_outcomes[(parsed.outcome * 2.0) as usize] += 1;
            stats.teacher_rows += usize::from(parsed.teacher);
            for (side, features) in [&parsed.white, &parsed.black].into_iter().enumerate() {
                stats.king_bucket_rows[side][usize::from(features[0]) / (PIECE_PLANES * 64)] += 1;
                for &feature in features {
                    stats.feature_support[usize::from(feature)] += 1;
                }
            }
            writer
                .write_all(parsed.row.as_bytes())
                .map_err(|error| error.to_string())?;
            keys.canonical.insert(parsed.key);
            keys.identity.insert(parsed.identity);
        }
    }
    if stats.raw_rows == 0 {
        return Err(format!("{}: input contains no positions", source.display()));
    }
    if stats.rows == 0 {
        return Err(format!("{}: every row was dropped", source.display()));
    }
    writer.flush().map_err(|error| error.to_string())?;
    Ok(stats)
}

pub fn json_string(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len() + 2);
    escaped.push('"');
    for character in text.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\t' => escaped.push_str("\\t"),
            control if (control as u32) < 0x20 => {
                let _ = write!(escaped, "\\u{:04x}", control as u32);
            }
            other => escaped.push(other),
        }
    }
    escaped.push('"');
    escaped
}

/// JSON description of the fixed architecture shared by every artifact.
pub fn architecture_json() -> String {
    format!(
        "{{\"version\": 1, \"features\": {INPUT_FEATURES}, \"hidden\": {HIDDEN_SIZE}, \"activation\": {ACTIVATION_MAX}, \"output_scale\": {OUTPUT_SCALE}}}"
    )
}

/// Hash of the running helper, which owns the feature mapping and scoring.
pub fn helper_sha256() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    sha256::file(&exe).map_err(|error| format!("{}: {error}", exe.display()))
}

/// Prepares both splits into a new output directory and returns the summary.
pub fn prepare(options: &Options) -> Result<String, String> {
    if options.output.exists() {
        return Err("output already exists; choose a new directory".to_owned());
    }
    for path in [&options.training, &options.development] {
        if !path.is_file() {
            return Err(format!("{}: not a regular file", path.display()));
        }
    }
    let helper = helper_sha256()?;
    let sources = [
        ("training", &options.training),
        ("development", &options.development),
    ];
    let source_hashes = sources
        .iter()
        .map(|(_, path)| {
            sha256::file_tree(path).map_err(|error| format!("{}: {error}", path.display()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let staging = options.output.with_extension("staging");
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(|error| error.to_string())?;
    }
    fs::create_dir_all(&staging).map_err(|error| error.to_string())?;
    let mut training_keys = Keys::default();
    let training_stats = process_split(
        &options.training,
        &staging.join("training.tsv"),
        &mut training_keys,
        None,
        options.deduplicate,
        false,
    )?;
    let mut development_keys = Keys::default();
    let development_stats = process_split(
        &options.development,
        &staging.join("development.tsv"),
        &mut development_keys,
        Some(&training_keys),
        options.deduplicate,
        options.drop_overlap,
    )?;
    drop((training_keys, development_keys));
    // Inputs are re-hashed after the pass: a corpus rewritten underneath the
    // helper would otherwise be bound to the wrong content.
    for ((_, path), before) in sources.iter().zip(&source_hashes) {
        let after = sha256::file_tree(path).map_err(|error| error.to_string())?;
        if after != *before {
            return Err("inputs changed during preparation".to_owned());
        }
    }
    let mut checksums = String::new();
    let mut manifest = format!(
        "{{\n  \"schema_version\": 2,\n  \"architecture\": {},\n  \"helper_sha256\": \"{helper}\",\n  \"deduplicate\": {},\n  \"drop_development_overlap\": {},\n  \"split_policy\": \"Reject canonical and feature-identical overlap, including turn changes and colour/rank mirrors; not a game/family independence certificate.\",\n  \"splits\": {{\n",
        architecture_json(),
        options.deduplicate,
        options.drop_overlap
    );
    let mut summary = String::from("{");
    for (index, ((name, source), (source_hash, stats))) in sources
        .iter()
        .zip(
            source_hashes
                .iter()
                .zip([&training_stats, &development_stats]),
        )
        .enumerate()
    {
        let file = format!("{name}.tsv");
        let prepared =
            sha256::file_tree(&staging.join(&file)).map_err(|error| error.to_string())?;
        let _ = writeln!(checksums, "{prepared}  {file}");
        let separator = if index == 0 { "" } else { ",\n" };
        let _ = write!(
            manifest,
            "{separator}    \"{name}\": {{\"source\": {}, \"source_checksum\": \"{source_hash}\", \"prepared_file\": \"{file}\", \"prepared_checksum\": \"{prepared}\", \"stats\": {}}}",
            json_string(&source.to_string_lossy()),
            stats.json(true)
        );
        let _ = write!(
            summary,
            "{}\"{name}\": {}",
            if index == 0 { "" } else { ", " },
            stats.json(false)
        );
    }
    manifest.push_str("\n  }\n}\n");
    summary.push('}');
    fs::write(staging.join("CHECKSUMS"), checksums).map_err(|error| error.to_string())?;
    fs::write(staging.join("helper.sha256"), format!("{helper}\n"))
        .map_err(|error| error.to_string())?;
    fs::write(staging.join("manifest.json"), manifest).map_err(|error| error.to_string())?;
    if options.output.exists() {
        return Err("output appeared during preparation".to_owned());
    }
    fs::rename(&staging, &options.output).map_err(|error| error.to_string())?;
    Ok(summary)
}

/// Verifies `CHECKSUMS` of a prepared dataset directory.
///
/// Returns the recorded file digests by name. The helper that prepared the
/// dataset is recorded in `helper.sha256` for provenance only: the trainer
/// recomputes every row's features from its FEN, which is the check that
/// matters, and a trainer-only change must not invalidate a corpus.
pub fn verify_dataset(directory: &Path) -> Result<BTreeMap<String, String>, String> {
    let checksums = fs::read_to_string(directory.join("CHECKSUMS"))
        .map_err(|error| format!("{}: CHECKSUMS: {error}", directory.display()))?;
    let mut hashes = BTreeMap::new();
    for line in checksums.lines() {
        let (expected, name) = line.split_once("  ").ok_or("malformed CHECKSUMS line")?;
        if name.contains('/') || name.contains("..") {
            return Err("unsafe prepared filename".to_owned());
        }
        let actual =
            sha256::file_tree(&directory.join(name)).map_err(|error| format!("{name}: {error}"))?;
        if actual != expected {
            return Err(format!("{name}: dataset checksum mismatch"));
        }
        hashes.insert(name.to_owned(), actual);
    }
    for required in ["training.tsv", "development.tsv"] {
        if !hashes.contains_key(required) {
            return Err(format!("CHECKSUMS does not cover {required}"));
        }
    }
    Ok(hashes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEN: &str = "4k3/8/5n2/8/8/8/2P5/4K3 w - - 0 1";

    fn pawn_on(file: usize) -> String {
        format!("4k3/8/5n2/8/8/8/{file}P{}/4K3 w - - 0 1", 7 - file)
    }

    fn run(
        training: &str,
        development: &str,
        deduplicate: bool,
        drop_overlap: bool,
    ) -> Result<(PathBuf, String), String> {
        let root = std::env::temp_dir().join(format!(
            "nnue-prepare-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("training.txt"), training).unwrap();
        fs::write(root.join("development.txt"), development).unwrap();
        let options = Options {
            training: root.join("training.txt"),
            development: root.join("development.txt"),
            output: root.join("prepared"),
            deduplicate,
            drop_overlap,
        };
        prepare(&options).map(|summary| (root, summary))
    }

    #[test]
    fn rows_carry_runtime_features_and_white_labels() {
        let (root, summary) = run(
            &format!("{FEN};0.5;-123\n"),
            &format!("{};1;150\n", pawn_on(3)),
            false,
            false,
        )
        .unwrap();
        let text = fs::read_to_string(root.join("prepared/training.tsv")).unwrap();
        assert!(text.starts_with(HEADER.as_str()));
        assert!(text.ends_with("\t0\t0.5\t-123\t781,1091,1258,1531\t850,1091,1205,1531\n"));
        assert!(summary.contains("\"training\": {\"raw_rows\": 1, \"rows\": 1, \"white_to_move\": 1, \"white_outcomes\": {\"0.0\": 0, \"0.5\": 1, \"1.0\": 0}, \"teacher_rows\": 1"));
        let hashes = verify_dataset(&root.join("prepared")).unwrap();
        assert_eq!(hashes.len(), 2);
        fs::write(root.join("prepared/training.tsv"), format!("{text}x")).unwrap();
        assert!(
            verify_dataset(&root.join("prepared"))
                .unwrap_err()
                .contains("checksum mismatch")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn duplicates_and_overlap_are_explicit() {
        let clocks = FEN.replace("0 1", "2 9");
        let training = format!("{FEN};1;150\n{clocks};1;150\n{};1;150\n", pawn_on(3));
        assert!(
            run(&training, &format!("{};1\n", pawn_on(4)), false, false)
                .unwrap_err()
                .contains("canonical_duplicate")
        );
        let (root, summary) = run(&training, &format!("{};1\n", pawn_on(4)), true, false).unwrap();
        assert!(summary.contains("\"dropped\": {\"canonical_duplicate\": 1}"));
        fs::remove_dir_all(root).unwrap();
        // The same pieces from Black's side of the board are the same features.
        let mirror = "4k3/2p5/8/8/8/5N2/8/4K3 b - - 0 1";
        assert!(
            run(
                &format!("{FEN};1;150\n{mirror};0\n"),
                &format!("{};1\n", pawn_on(4)),
                false,
                false
            )
            .unwrap_err()
            .contains("feature_duplicate")
        );
        assert!(
            run(
                &format!("{FEN};1;150\n"),
                &format!("{mirror};0\n"),
                false,
                false
            )
            .unwrap_err()
            .contains("overlap")
        );
        let (root, summary) = run(
            &format!("{FEN};1;150\n"),
            &format!("{mirror};0\n{};1\n", pawn_on(4)),
            false,
            true,
        )
        .unwrap();
        assert!(
            summary.contains("\"dropped\": {\"training_overlap\": 1}"),
            "{summary}"
        );
        fs::remove_dir_all(root).unwrap();
        assert!(
            run(
                &format!("{FEN};1;150\n"),
                &format!("{mirror};0\n"),
                false,
                true
            )
            .unwrap_err()
            .contains("every row was dropped")
        );
    }
}
