//! Multi-threaded CPU trainer for the fixed Jakgro NNUE architecture.
//!
//! The float model mirrors the engine's integer one exactly: unclipped sums
//! are `hidden + Σ input[feature]`, activations clip to `0..=1` and are
//! squared, the score is `400 * (Σ output · activation² + bias)` centipawns,
//! and the loss is the mean squared error between the sigmoid of that score
//! and a label mixing the game outcome with the teacher's own sigmoid.
//! Full-parameter Adam runs over seeded minibatches; the exported integers
//! are the engine's quantization (feature transformer times 255, output
//! weights times 64, output bias times 64 * 255 * 255), and every published
//! epoch is re-read through the engine's own [`Network`] loader for its
//! integer metrics.
//!
//! Determinism does not depend on the thread count: a minibatch is split into
//! a fixed number of shards whose partial gradients are reduced in shard
//! order, and every per-thread reduction covers a fixed parameter range.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Instant;

use crate::{prepare, sha256};
use cozy_chess::Board;
use jakgro::engine::nnue::{
    ACTIVATION_MAX, FEATURE_SET, FILE_BYTES, FORMAT_VERSION, HEADER_BYTES, HIDDEN_SIZE,
    INPUT_FEATURES, MAGIC, MAX_SCORE, Network, OUTPUT_BUCKETS, OUTPUT_SCALE, OUTPUT_UNIT,
    OUTPUT_WEIGHT_LIMIT, SCORE_SCALE, active_features,
};

/// Centipawns per unit of float output.
const FLOAT_CP_SCALE: f32 = SCORE_SCALE as f32;
/// Fixed minibatch shard count; partial gradients reduce in this order.
const SHARDS: usize = 16;
/// Largest training subsample scored per epoch for the improvement rule.
const TRAIN_METRIC_ROWS: usize = 65_536;
const MAX_PIECES: usize = 32;

#[derive(Clone, Debug)]
pub struct Options {
    pub epochs: usize,
    pub batch_size: usize,
    pub rate: f32,
    /// Multiplies the learning rate after every epoch.
    pub rate_decay: f32,
    pub l2: f32,
    pub seed: u64,
    pub label_mix: f32,
    pub k: f32,
    pub threads: usize,
    /// An exported network whose weights the run starts from, instead of the
    /// seeded random initialization.
    pub init_network: Option<PathBuf>,
    /// Per-step decay of an exponential moving average of the weights. When
    /// set, the averaged weights are exported and selected as `network.nnue`
    /// and the best raw epoch is kept beside them as `raw-network.nnue`; the
    /// raw trajectory itself does not depend on it.
    pub ema: Option<f32>,
}

impl Options {
    pub fn parse(arguments: &[String]) -> Result<(Self, PathBuf, PathBuf), String> {
        let mut values = BTreeMap::new();
        let mut index = 0;
        while index < arguments.len() {
            let key = arguments[index].as_str();
            let value = arguments
                .get(index + 1)
                .ok_or_else(|| format!("{key} needs a value"))?;
            if !key.starts_with("--") || values.insert(key, value.as_str()).is_some() {
                return Err(format!("unexpected or repeated option {key}"));
            }
            index += 2;
        }
        fn number<T: std::str::FromStr>(
            values: &BTreeMap<&str, &str>,
            key: &str,
            default: T,
        ) -> Result<T, String> {
            values.get(key).map_or(Ok(default), |value| {
                value
                    .parse()
                    .map_err(|_| format!("{key}: invalid value {value}"))
            })
        }
        let data = PathBuf::from(values.get("--data-dir").ok_or("train needs --data-dir")?);
        let output = PathBuf::from(
            values
                .get("--output-dir")
                .ok_or("train needs --output-dir")?,
        );
        let options = Self {
            epochs: number(&values, "--epochs", 10)?,
            batch_size: number(&values, "--batch-size", 256)?,
            rate: number(&values, "--rate", 0.001)?,
            rate_decay: number(&values, "--rate-decay", 1.0)?,
            l2: number(&values, "--l2", 1e-6)?,
            seed: number(&values, "--seed", 75)?,
            label_mix: number(&values, "--lambda", 0.5)?,
            k: number(&values, "--k", 0.880_682_5)?,
            threads: number(
                &values,
                "--threads",
                thread::available_parallelism().map_or(1, |count| count.get().min(32)),
            )?,
            init_network: values.get("--init-network").map(PathBuf::from),
            ema: values
                .get("--ema")
                .map(|value| {
                    value
                        .parse()
                        .map_err(|_| format!("--ema: invalid value {value}"))
                })
                .transpose()?,
        };
        for key in values.keys() {
            if ![
                "--data-dir",
                "--output-dir",
                "--epochs",
                "--batch-size",
                "--rate",
                "--rate-decay",
                "--l2",
                "--seed",
                "--lambda",
                "--k",
                "--threads",
                "--init-network",
                "--ema",
            ]
            .contains(key)
            {
                return Err(format!("unknown option {key}"));
            }
        }
        options.validate()?;
        Ok((options, data, output))
    }

    fn validate(&self) -> Result<(), String> {
        let check = |condition: bool, message: &str| {
            if condition {
                Ok(())
            } else {
                Err(message.to_owned())
            }
        };
        check(
            (1..=10_000).contains(&self.epochs),
            "epochs must be 1..10000; initialization is not a trained model",
        )?;
        check(
            (1..=65_536).contains(&self.batch_size),
            "batch size must be 1..65536",
        )?;
        check(
            self.rate.is_finite() && self.rate > 0.0 && self.rate <= 1.0,
            "rate must be finite in (0,1]",
        )?;
        check(
            self.rate_decay.is_finite() && self.rate_decay > 0.0 && self.rate_decay <= 1.0,
            "rate decay must be finite in (0,1]",
        )?;
        check(
            self.l2.is_finite() && (0.0..=1.0).contains(&self.l2),
            "L2 must be finite in [0,1]",
        )?;
        check(
            self.label_mix.is_finite() && (0.0..=1.0).contains(&self.label_mix),
            "lambda must be in [0,1]",
        )?;
        check(
            self.k.is_finite() && self.k > 0.0 && self.k <= 10.0,
            "K must be finite in (0,10]",
        )?;
        check((1..=256).contains(&self.threads), "threads must be 1..256")?;
        check(
            self.ema
                .is_none_or(|decay| decay.is_finite() && decay > 0.0 && decay < 1.0),
            "EMA decay must be finite in (0,1)",
        )
    }
}

/// One prepared position's training inputs, oriented to the side to move.
///
/// A training row carries no board: gradients need only the features, and a
/// board per row would more than double what a large corpus holds in memory.
#[derive(Clone)]
struct Row {
    /// Side-to-move perspective features, then the opponent's.
    features: [[u16; MAX_PIECES]; 2],
    count: u8,
    /// Output layer selected by the piece count.
    bucket: u8,
    /// Fitted label after mixing outcome and teacher probability.
    label: f32,
}

/// What integer metrics need of a position: the board the engine's own
/// network evaluates, and both targets.
#[derive(Clone)]
struct MetricRow {
    board: Board,
    /// Side-to-move outcome in points.
    outcome: f32,
    /// Fitted label after mixing outcome and teacher probability.
    label: f32,
}

/// A prepared split as the trainer keeps it.
struct Split {
    /// Training inputs of every row; empty for a split read for metrics only.
    rows: Vec<Row>,
    /// Metric inputs of the rows [`Keep`] selects, in file order.
    metrics: Vec<MetricRow>,
}

/// Which rows of a split are kept, and in which form.
#[derive(Clone, Copy)]
enum Keep {
    /// Every row, for integer metrics only: the development split.
    Metrics,
    /// Every row's training inputs, and the metric inputs of the rows
    /// [`strided`] selects for at most `metric_rows`.
    Training { metric_rows: usize },
}

/// Bytes read per block when streaming a split; blocks end on a newline.
const BLOCK_BYTES: usize = 1 << 29;

fn sigmoid_scale(k: f32) -> f32 {
    std::f32::consts::LN_10 * k / 400.0
}

fn sigmoid(score: f32, k: f32) -> f32 {
    1.0 / (1.0 + (-(score * sigmoid_scale(k)).clamp(-80.0, 80.0)).exp())
}

/// Indices of at most `maximum` rows spread evenly over `len` rows, the
/// first and the last included, in increasing order.
fn strided(len: usize, maximum: usize) -> Vec<usize> {
    if len <= maximum {
        return (0..len).collect();
    }
    (0..maximum)
        .map(|index| index * (len - 1) / (maximum - 1))
        .collect()
}

/// The number of lines in a block of whole lines, the last of which may lack
/// its newline.
fn line_count(block: &[u8]) -> usize {
    block.iter().filter(|&&byte| byte == b'\n').count()
        + usize::from(block.last().is_some_and(|&byte| byte != b'\n'))
}

/// Validates a split's two header lines and returns the offset of its body.
///
/// The schema line must name this feature contract (see
/// [`crate::schema_accepted`]) and the column line must match exactly.
fn check_header(path: &Path, bytes: &[u8]) -> Result<usize, String> {
    let mut offset = 0;
    for line in 0..2 {
        let end = bytes[offset..]
            .iter()
            .position(|&byte| byte == b'\n')
            .map(|end| offset + end)
            .ok_or_else(|| format!("{}: truncated header", path.display()))?;
        let text = std::str::from_utf8(&bytes[offset..end]).unwrap_or_default();
        let accepted = if line == 0 {
            crate::schema_accepted(text)
        } else {
            text == crate::COLUMNS
        };
        if !accepted {
            let what = if line == 0 {
                "unsupported feature schema"
            } else {
                "wrong dataset columns"
            };
            return Err(format!("{}: {what}", path.display()));
        }
        offset = end + 1;
    }
    Ok(offset)
}

/// Streams a prepared split's body in blocks of whole lines of at most
/// `block_bytes`, after validating its header.
///
/// `consume` receives every block with the number of body lines before it;
/// only the file's last line may lack its newline. Returns the number of body
/// lines. Memory is bounded by the block, whatever the size of the split.
fn stream_body(
    path: &Path,
    block_bytes: usize,
    mut consume: impl FnMut(&[u8], usize) -> Result<(), String>,
) -> Result<usize, String> {
    let failed = |error: std::io::Error| format!("{}: {error}", path.display());
    let mut file = fs::File::open(path).map_err(failed)?;
    let mut buffer = vec![0_u8; block_bytes];
    let mut filled = 0;
    let mut body = None;
    let mut lines = 0;
    loop {
        let read = crate::fill(&mut file, &mut buffer[filled..]).map_err(failed)?;
        let finished = filled + read < buffer.len();
        filled += read;
        let start = match body {
            Some(start) => start,
            None => check_header(path, &buffer[..filled])?,
        };
        body = Some(0);
        let end = if finished {
            filled
        } else {
            buffer[start..filled]
                .iter()
                .rposition(|&byte| byte == b'\n')
                .map(|last| start + last + 1)
                .ok_or_else(|| format!("{}: a line exceeds {block_bytes} bytes", path.display()))?
        };
        let block = &buffer[start..end];
        if !block.is_empty() {
            consume(block, lines)?;
            lines += line_count(block);
        }
        if finished {
            return Ok(lines);
        }
        buffer.copy_within(end..filled, 0);
        filled -= end;
    }
}

/// Parses one block of whole lines on every thread.
///
/// `first_line` is the number of body lines before the block and `selected`
/// the sorted body-line indices whose metric inputs a training split keeps.
/// Returns the block's kept rows and metric rows, in file order.
#[allow(clippy::too_many_arguments)]
fn parse_block(
    path: &Path,
    block: &[u8],
    first_line: usize,
    label_mix: f32,
    k: f32,
    threads: usize,
    keep: Keep,
    selected: &[usize],
) -> Result<(Vec<Row>, Vec<MetricRow>), String> {
    // Chunk boundaries fall on newlines so every line is parsed exactly once.
    let target = block.len().div_ceil(threads * 4).max(1);
    let mut bounds = vec![0];
    while *bounds.last().unwrap() < block.len() {
        let start = *bounds.last().unwrap();
        let end = (start + target).min(block.len());
        let end = block[end..]
            .iter()
            .position(|&byte| byte == b'\n')
            .map_or(block.len(), |extra| end + extra + 1);
        bounds.push(end);
    }
    let mut lines_before = vec![first_line];
    for pair in bounds.windows(2) {
        let last = *lines_before.last().unwrap();
        lines_before.push(last + line_count(&block[pair[0]..pair[1]]));
    }
    let chunks = thread::scope(|scope| {
        let handles = bounds
            .windows(2)
            .zip(&lines_before)
            .map(|(pair, &chunk_first)| {
                let chunk = &block[pair[0]..pair[1]];
                scope.spawn(move || -> Result<(Vec<Row>, Vec<MetricRow>), String> {
                    let text = std::str::from_utf8(chunk)
                        .map_err(|_| format!("{}: invalid UTF-8", path.display()))?;
                    let mut rows = Vec::new();
                    let mut metrics = Vec::new();
                    let mut next = selected.partition_point(|&line| line < chunk_first);
                    for (index, line) in text.lines().enumerate() {
                        let body_line = chunk_first + index;
                        let (row, metric) = parse_row(line, label_mix, k).map_err(|error| {
                            format!("{}:{}: {error}", path.display(), body_line + 3)
                        })?;
                        match keep {
                            Keep::Metrics => metrics.push(metric),
                            Keep::Training { .. } => {
                                rows.push(row);
                                if selected.get(next) == Some(&body_line) {
                                    metrics.push(metric);
                                    next += 1;
                                }
                            }
                        }
                    }
                    Ok((rows, metrics))
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("row parser panicked"))
            .collect::<Result<Vec<_>, _>>()
    })?;
    let mut rows = Vec::new();
    let mut metrics = Vec::new();
    for (chunk_rows, chunk_metrics) in chunks {
        rows.extend(chunk_rows);
        metrics.extend(chunk_metrics);
    }
    Ok((rows, metrics))
}

/// Loads a prepared split, streaming it twice: once to count its rows, so
/// storage is reserved exactly and a training split's metric rows are known
/// before any is parsed, then to parse every block on every thread.
///
/// Every row's stored features are recomputed from its FEN with the engine's
/// own mapping, so a dataset prepared by another feature set is rejected here
/// rather than trusted.
fn read_split(
    path: &Path,
    block_bytes: usize,
    label_mix: f32,
    k: f32,
    threads: usize,
    keep: Keep,
) -> Result<Split, String> {
    let total = stream_body(path, block_bytes, |_, _| Ok(()))?;
    if total == 0 {
        return Err(format!("{}: empty dataset", path.display()));
    }
    let selected = match keep {
        Keep::Metrics => Vec::new(),
        Keep::Training { metric_rows } => strided(total, metric_rows),
    };
    let mut split = Split {
        rows: Vec::new(),
        metrics: Vec::new(),
    };
    match keep {
        Keep::Metrics => split.metrics.reserve_exact(total),
        Keep::Training { .. } => {
            split.rows.reserve_exact(total);
            split.metrics.reserve_exact(selected.len());
        }
    }
    let parsed = stream_body(path, block_bytes, |block, first_line| {
        let (rows, metrics) = parse_block(
            path, block, first_line, label_mix, k, threads, keep, &selected,
        )?;
        split.rows.extend(rows);
        split.metrics.extend(metrics);
        Ok(())
    })?;
    let (rows, metrics) = match keep {
        Keep::Metrics => (0, total),
        Keep::Training { .. } => (total, selected.len()),
    };
    if parsed != total || split.rows.len() != rows || split.metrics.len() != metrics {
        return Err(format!("{}: changed while it was read", path.display()));
    }
    Ok(split)
}

fn parse_features(text: &str) -> Result<([u16; MAX_PIECES], usize), String> {
    let mut features = [0_u16; MAX_PIECES];
    let mut count = 0;
    for value in text.split(',') {
        let value: u16 = value
            .parse()
            .map_err(|_| format!("invalid feature index {value}"))?;
        if usize::from(value) >= INPUT_FEATURES
            || count >= MAX_PIECES
            || (count > 0 && features[count - 1] >= value)
        {
            return Err("features must be sorted, unique and in range".to_owned());
        }
        features[count] = value;
        count += 1;
    }
    if count == 0 {
        return Err("empty feature vector".to_owned());
    }
    Ok((features, count))
}

fn parse_row(line: &str, label_mix: f32, k: f32) -> Result<(Row, MetricRow), String> {
    let fields = line.split('\t').collect::<Vec<_>>();
    let [fen, _key, stm, outcome, teacher, white, black] = fields.as_slice() else {
        return Err("expected seven fields".to_owned());
    };
    let board = fen.parse::<Board>().map_err(|_| "invalid FEN".to_owned())?;
    let stm: u8 = stm.parse().map_err(|_| "invalid turn".to_owned())?;
    let white_outcome: f32 = outcome.parse().map_err(|_| "invalid outcome".to_owned())?;
    if stm > 1 || ![0.0, 0.5, 1.0].contains(&white_outcome) {
        return Err("invalid turn or outcome".to_owned());
    }
    if (stm == 1) != (board.side_to_move() == cozy_chess::Color::Black) {
        return Err("turn disagrees with FEN".to_owned());
    }
    let teacher = if *teacher == "-" {
        None
    } else {
        let score: f32 = teacher
            .parse()
            .map_err(|_| "invalid teacher score".to_owned())?;
        if !score.is_finite() || score.abs() > 32_000.0 {
            return Err("invalid teacher score".to_owned());
        }
        Some(score)
    };
    let (white, white_count) = parse_features(white)?;
    let (black, black_count) = parse_features(black)?;
    if white_count != black_count {
        return Err("perspective piece counts differ".to_owned());
    }
    for (perspective, stored, count) in [
        (cozy_chess::Color::White, &white, white_count),
        (cozy_chess::Color::Black, &black, black_count),
    ] {
        if active_features(&board, perspective) != stored[..count] {
            return Err("stored features disagree with the engine's feature mapping".to_owned());
        }
    }
    let sign = if stm == 0 { 1.0 } else { -1.0 };
    let outcome = if stm == 0 {
        white_outcome
    } else {
        1.0 - white_outcome
    };
    let label = teacher.map_or(outcome, |score| {
        label_mix * outcome + (1.0 - label_mix) * sigmoid(sign * score, k)
    });
    Ok((
        Row {
            features: if stm == 0 {
                [white, black]
            } else {
                [black, white]
            },
            count: white_count as u8,
            bucket: ((white_count - 1) / 4) as u8,
            label,
        },
        MetricRow {
            board,
            outcome,
            label,
        },
    ))
}

/// SplitMix64: a small seeded generator for initialization and shuffling.
struct Random(u64);

impl Random {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1_u64 << 53) as f64
    }

    fn normal(&mut self, deviation: f64) -> f32 {
        let u = self.uniform().max(f64::MIN_POSITIVE);
        let v = self.uniform();
        ((-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos() * deviation) as f32
    }

    fn shuffle<T>(&mut self, values: &mut [T]) {
        for index in (1..values.len()).rev() {
            let other = (self.next_u64() % (index as u64 + 1)) as usize;
            values.swap(index, other);
        }
    }
}

/// Float parameters in the reference layout; `input` is feature-major.
#[derive(Clone)]
struct Parameters {
    input: Vec<f32>,
    hidden: Vec<f32>,
    /// Per output bucket: side-to-move row, then opponent row.
    output: Vec<f32>,
    bias: Vec<f32>,
}

impl Parameters {
    fn zeros() -> Self {
        Self {
            input: vec![0.0; INPUT_FEATURES * HIDDEN_SIZE],
            hidden: vec![0.0; HIDDEN_SIZE],
            output: vec![0.0; OUTPUT_BUCKETS * 2 * HIDDEN_SIZE],
            bias: vec![0.0; OUTPUT_BUCKETS],
        }
    }

    fn initialize(support: &[bool], seed: u64) -> Self {
        let mut random = Random(seed);
        let mut parameters = Self::zeros();
        for (feature, row) in parameters.input.chunks_mut(HIDDEN_SIZE).enumerate() {
            for value in row {
                let sample = random.normal(0.02);
                if support[feature] {
                    *value = sample;
                }
            }
        }
        parameters.hidden.fill(0.1);
        for value in &mut parameters.output {
            *value = random.normal(0.01);
        }
        parameters
    }

    /// Recovers float parameters from an exported network, so a run can
    /// continue that network's training on another corpus.
    ///
    /// The inverse of `quantize`: every weight is divided by the scale it was
    /// rounded with, so the recovered parameters differ from the ones that
    /// were exported by at most half a quantization step. The engine's loader
    /// is the authority on whether the bytes are a network at all.
    fn dequantize(bytes: &[u8]) -> Result<Self, String> {
        Network::from_bytes(bytes).map_err(|error| format!("initial network rejected: {error}"))?;
        let payload = &bytes[HEADER_BYTES..];
        let read_i16 = |offset: usize| i16::from_le_bytes([payload[offset], payload[offset + 1]]);
        let activation = ACTIVATION_MAX as f32;
        let mut parameters = Self::zeros();
        for (unit, value) in parameters.hidden.iter_mut().enumerate() {
            *value = f32::from(read_i16(2 * unit)) / activation;
        }
        let input_start = 2 * HIDDEN_SIZE;
        for (index, value) in parameters.input.iter_mut().enumerate() {
            *value = f32::from(read_i16(input_start + 2 * index)) / activation;
        }
        let output_start = input_start + 2 * INPUT_FEATURES * HIDDEN_SIZE;
        for (index, value) in parameters.output.iter_mut().enumerate() {
            *value = f32::from(read_i16(output_start + 2 * index)) / OUTPUT_SCALE as f32;
        }
        let bias_start = output_start + 2 * OUTPUT_BUCKETS * 2 * HIDDEN_SIZE;
        for (bucket, value) in parameters.bias.iter_mut().enumerate() {
            let offset = bias_start + 4 * bucket;
            let bias = i32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap());
            *value = bias as f32 / OUTPUT_UNIT as f32;
        }
        Ok(parameters)
    }

    /// Marks every feature whose row carries weight, so a row an initial
    /// network learned is kept although this corpus never shows the feature.
    fn extend_support(&self, support: &mut [bool]) {
        for (feature, row) in self.input.chunks(HIDDEN_SIZE).enumerate() {
            if row.iter().any(|&weight| weight != 0.0) {
                support[feature] = true;
            }
        }
    }

    /// Unclipped sums, activations and the raw float score of one row.
    #[inline(always)]
    fn forward(&self, row: &Row) -> ([[f32; HIDDEN_SIZE]; 2], f32) {
        let mut sums = [[0.0_f32; HIDDEN_SIZE]; 2];
        let bucket = usize::from(row.bucket);
        let mut total = self.bias[bucket];
        for (side, sum) in sums.iter_mut().enumerate() {
            sum.copy_from_slice(&self.hidden);
            for &feature in &row.features[side][..usize::from(row.count)] {
                let start = usize::from(feature) * HIDDEN_SIZE;
                for (value, &weight) in sum.iter_mut().zip(&self.input[start..start + HIDDEN_SIZE])
                {
                    *value += weight;
                }
            }
            let start = (bucket * 2 + side) * HIDDEN_SIZE;
            for (&value, &weight) in sum.iter().zip(&self.output[start..start + HIDDEN_SIZE]) {
                let activation = value.clamp(0.0, 1.0);
                total += activation * activation * weight;
            }
        }
        (sums, FLOAT_CP_SCALE * total)
    }

    fn quantize(&self, support: &[bool]) -> Result<Vec<u8>, String> {
        fn scale_i16(value: f32, scale: f32) -> Result<i16, String> {
            let scaled = f64::from(value) * f64::from(scale);
            if !scaled.is_finite() {
                return Err("nonfinite parameter during export".to_owned());
            }
            let rounded = scaled.round();
            if rounded < f64::from(i16::MIN) || rounded > f64::from(i16::MAX) {
                return Err("quantized parameter exceeds storage bounds".to_owned());
            }
            Ok(rounded as i16)
        }
        let activation = ACTIVATION_MAX as f32;
        let mut payload = Vec::with_capacity(FILE_BYTES - HEADER_BYTES);
        for &value in &self.hidden {
            payload.extend_from_slice(&scale_i16(value, activation)?.to_le_bytes());
        }
        for (feature, row) in self.input.chunks(HIDDEN_SIZE).enumerate() {
            for &value in row {
                let quantized = scale_i16(value, activation)?;
                if !support[feature] && quantized != 0 {
                    return Err("unobserved feature rows must remain zero".to_owned());
                }
                payload.extend_from_slice(&quantized.to_le_bytes());
            }
        }
        for &value in &self.output {
            payload.extend_from_slice(&scale_i16(value, OUTPUT_SCALE as f32)?.to_le_bytes());
        }
        for &value in &self.bias {
            let bias = (f64::from(value) * f64::from(OUTPUT_UNIT)).round();
            if !bias.is_finite() || bias < f64::from(i32::MIN) || bias > f64::from(i32::MAX) {
                return Err("output bias exceeds storage bounds".to_owned());
            }
            payload.extend_from_slice(&(bias as i32).to_le_bytes());
        }
        let mut bytes = Vec::with_capacity(FILE_BYTES);
        bytes.extend_from_slice(&MAGIC);
        for field in [
            FORMAT_VERSION,
            FEATURE_SET,
            INPUT_FEATURES as u32,
            HIDDEN_SIZE as u32,
            ACTIVATION_MAX as u32,
            OUTPUT_SCALE as u32,
            payload.len() as u32,
            OUTPUT_BUCKETS as u32,
        ] {
            bytes.extend_from_slice(&field.to_le_bytes());
        }
        let checksum = payload
            .iter()
            .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
                (hash ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3)
            });
        bytes.extend_from_slice(&checksum.to_le_bytes());
        bytes.extend_from_slice(&payload);
        // The engine's loader is the authority on what was exported.
        Network::from_bytes(&bytes)
            .map_err(|error| format!("exported network rejected: {error}"))?;
        Ok(bytes)
    }
}

/// Zeroes a shard, then accumulates its rows' batch-mean gradient into it.
///
/// Returns the shard's sum of squared errors. The arithmetic runs at the
/// host's vector width when AVX2 and FMA are available; the result is the
/// same either way up to the usual float reassociation the autovectorizer
/// performs, which is fixed for a given binary and host.
fn shard_gradient(
    parameters: &Parameters,
    rows: &[&Row],
    batch: usize,
    k: f32,
    gradient: &mut Parameters,
) -> f64 {
    #[cfg(target_arch = "x86_64")]
    if wide_math() {
        // SAFETY: `wide_math` checked the CPU features this function requires.
        return unsafe { shard_gradient_avx2(parameters, rows, batch, k, gradient) };
    }
    shard_gradient_impl(parameters, rows, batch, k, gradient)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn shard_gradient_avx2(
    parameters: &Parameters,
    rows: &[&Row],
    batch: usize,
    k: f32,
    gradient: &mut Parameters,
) -> f64 {
    shard_gradient_impl(parameters, rows, batch, k, gradient)
}

/// Whether the AVX2/FMA paths may run on this host, decided once.
fn wide_math() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        static WIDE: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
            is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma")
        });
        *WIDE
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

#[inline(always)]
fn shard_gradient_impl(
    parameters: &Parameters,
    rows: &[&Row],
    batch: usize,
    k: f32,
    gradient: &mut Parameters,
) -> f64 {
    gradient.input.fill(0.0);
    gradient.hidden.fill(0.0);
    gradient.output.fill(0.0);
    gradient.bias.fill(0.0);
    let mut loss = 0.0_f64;
    let derivative_scale = 2.0 / batch as f32 * sigmoid_scale(k) * FLOAT_CP_SCALE;
    for row in rows {
        let (sums, raw) = parameters.forward(row);
        let score = raw.clamp(-(MAX_SCORE as f32), MAX_SCORE as f32);
        let prediction = sigmoid(score, k);
        let error = prediction - row.label;
        loss += f64::from(error * error);
        if raw.abs() >= MAX_SCORE as f32 {
            continue;
        }
        let outer = derivative_scale * error * prediction * (1.0 - prediction);
        let bucket = usize::from(row.bucket);
        gradient.bias[bucket] += outer;
        for (side, sums) in sums.iter().enumerate() {
            let start = (bucket * 2 + side) * HIDDEN_SIZE;
            let weights = &parameters.output[start..start + HIDDEN_SIZE];
            let mut hidden_gradient = [0.0_f32; HIDDEN_SIZE];
            for (unit, &sum) in sums.iter().enumerate() {
                let activation = sum.clamp(0.0, 1.0);
                gradient.output[start + unit] += outer * activation * activation;
                // d(activation²)/d(sum) is 2 * activation inside the clip and
                // zero outside it.
                if sum > 0.0 && sum < 1.0 {
                    hidden_gradient[unit] = outer * 2.0 * activation * weights[unit];
                }
            }
            for (value, &delta) in gradient.hidden.iter_mut().zip(&hidden_gradient) {
                *value += delta;
            }
            for &feature in &row.features[side][..usize::from(row.count)] {
                let start = usize::from(feature) * HIDDEN_SIZE;
                for (value, &delta) in gradient.input[start..start + HIDDEN_SIZE]
                    .iter_mut()
                    .zip(&hidden_gradient)
                {
                    *value += delta;
                }
            }
        }
    }
    loss
}

/// Full-parameter Adam with bias correction over a contiguous range.
///
/// Updated values are clamped to `limit`, the largest magnitude the integer
/// export can store for that tensor, so a long run cannot drift into weights
/// the engine could never load.
fn adam(
    parameters: &mut [f32],
    moment: &mut [f32],
    velocity: &mut [f32],
    gradient: impl Fn(usize) -> f32,
    update: AdamUpdate,
    limit: f32,
) {
    #[cfg(target_arch = "x86_64")]
    if wide_math() {
        // SAFETY: `wide_math` checked the CPU features this function requires.
        return unsafe { adam_avx2(parameters, moment, velocity, gradient, update, limit) };
    }
    adam_impl(parameters, moment, velocity, gradient, update, limit);
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn adam_avx2(
    parameters: &mut [f32],
    moment: &mut [f32],
    velocity: &mut [f32],
    gradient: impl Fn(usize) -> f32,
    update: AdamUpdate,
    limit: f32,
) {
    adam_impl(parameters, moment, velocity, gradient, update, limit);
}

#[inline(always)]
fn adam_impl(
    parameters: &mut [f32],
    moment: &mut [f32],
    velocity: &mut [f32],
    gradient: impl Fn(usize) -> f32,
    update: AdamUpdate,
    limit: f32,
) {
    let AdamUpdate { step, rate, l2 } = update;
    let moment_correction = 1.0 - 0.9_f32.powi(step as i32);
    let velocity_correction = 1.0 - 0.999_f32.powi(step as i32);
    // Moments of parameters that stop receiving gradient decay geometrically
    // into subnormal floats, which the CPU handles two orders of magnitude
    // slower; flushing them to zero changes no visible update.
    const TINY: f32 = 1e-30;
    let flush = |value: f32| if value.abs() < TINY { 0.0 } else { value };
    for index in 0..parameters.len() {
        let g = gradient(index) + l2 * parameters[index];
        moment[index] = flush(0.9 * moment[index] + 0.1 * g);
        velocity[index] = flush(0.999 * velocity[index] + 0.001 * g * g);
        parameters[index] = (parameters[index]
            - rate * (moment[index] / moment_correction)
                / ((velocity[index] / velocity_correction).sqrt() + 1e-8))
            .clamp(-limit, limit);
    }
}

/// Moves an exponential moving average one step toward `values`.
fn blend(average: &mut [f32], values: &[f32], decay: f32) {
    for (average, &value) in average.iter_mut().zip(values) {
        *average = decay * *average + (1.0 - decay) * value;
    }
}

/// The step-dependent Adam settings shared by every tensor in one update.
#[derive(Clone, Copy)]
struct AdamUpdate {
    step: usize,
    rate: f32,
    l2: f32,
}

/// Largest float magnitudes the i16/i32 export can represent per tensor.
/// Output weights stop at the loader's bound, not at `i16::MAX`.
const INPUT_LIMIT: f32 = i16::MAX as f32 / ACTIVATION_MAX as f32;
const OUTPUT_LIMIT: f32 = OUTPUT_WEIGHT_LIMIT as f32 / OUTPUT_SCALE as f32;
const BIAS_LIMIT: f32 = i32::MAX as f32 / OUTPUT_UNIT as f32;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Metrics {
    pub rows: usize,
    pub label_mse: f64,
    pub outcome_mse: f64,
    pub minimum_cp: i32,
    pub maximum_cp: i32,
}

impl Metrics {
    fn json(&self) -> String {
        format!(
            "{{\"label_mse\": {}, \"maximum_cp\": {}, \"minimum_cp\": {}, \"outcome_mse\": {}, \"rows\": {}}}",
            self.label_mse, self.maximum_cp, self.minimum_cp, self.outcome_mse, self.rows
        )
    }
}

/// Integer metrics through the engine's own evaluation of each board.
fn integer_metrics(network: &Network, rows: &[MetricRow], k: f32, threads: usize) -> Metrics {
    let chunk = rows.len().div_ceil(threads).max(1);
    let partials = thread::scope(|scope| {
        let handles = rows
            .chunks(chunk)
            .map(|rows| {
                scope.spawn(move || {
                    let mut metrics = Metrics {
                        rows: rows.len(),
                        minimum_cp: i32::MAX,
                        maximum_cp: i32::MIN,
                        ..Metrics::default()
                    };
                    for row in rows {
                        let cp = network.evaluate(&row.board);
                        let prediction = f64::from(sigmoid(cp as f32, k));
                        metrics.label_mse += (prediction - f64::from(row.label)).powi(2);
                        metrics.outcome_mse += (prediction - f64::from(row.outcome)).powi(2);
                        metrics.minimum_cp = metrics.minimum_cp.min(cp);
                        metrics.maximum_cp = metrics.maximum_cp.max(cp);
                    }
                    metrics
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("metric worker panicked"))
            .collect::<Vec<_>>()
    });
    let mut total = Metrics {
        minimum_cp: i32::MAX,
        maximum_cp: i32::MIN,
        ..Metrics::default()
    };
    for partial in partials {
        total.rows += partial.rows;
        total.label_mse += partial.label_mse;
        total.outcome_mse += partial.outcome_mse;
        total.minimum_cp = total.minimum_cp.min(partial.minimum_cp);
        total.maximum_cp = total.maximum_cp.max(partial.maximum_cp);
    }
    total.label_mse /= total.rows as f64;
    total.outcome_mse /= total.rows as f64;
    total
}

/// Raw pointer to state the epoch workers share under barrier discipline.
///
/// Between two barriers every worker either reads a buffer or writes a part of
/// it no other thread touches; the barriers order those phases. This is the
/// only place the trainer steps outside the borrow checker.
struct Ptr<T>(*mut T);

impl<T> Clone for Ptr<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Ptr<T> {}

// SAFETY: the pointee outlives the epoch scope and access follows the phase
// discipline documented on `run_epoch`.
unsafe impl<T> Send for Ptr<T> {}
unsafe impl<T> Sync for Ptr<T> {}

impl<T> Ptr<T> {
    /// Accessor so closures capture the whole wrapper rather than the field.
    fn get(self) -> *mut T {
        self.0
    }
}

struct EpochState<'a> {
    parameters: &'a mut Parameters,
    moment: &'a mut Parameters,
    velocity: &'a mut Parameters,
    shards: &'a mut [Parameters],
    /// The moving average `Options::ema` asks for, updated after every step.
    average: Option<&'a mut Parameters>,
}

/// What the workers need for one minibatch.
struct Step<'a> {
    rows: Vec<&'a Row>,
    shard_size: usize,
    update: AdamUpdate,
    finished: bool,
}

/// Runs every minibatch of one epoch on a persistent set of `SHARDS` workers.
///
/// Per step, three barriers separate: (1) the main thread publishing the
/// batch; (2) every worker accumulating its shard's gradient while reading
/// the parameters; (3) every worker applying Adam to its own parameter range
/// while reading all shards, then moving the same range of the average, if
/// any, toward it. Worker zero also owns the small tensors. Returns
/// the summed batch losses, the batch count and the wall seconds of phases
/// two and three.
fn run_epoch<'a>(
    training: &'a [Row],
    order: &[usize],
    options: &Options,
    state: EpochState<'_>,
    step: &mut usize,
    rate: f32,
) -> Result<(f64, usize, [f64; 2]), String> {
    let shard_count = state.shards.len();
    let barrier = std::sync::Barrier::new(shard_count + 1);
    let parameters = Ptr(state.parameters as *mut Parameters);
    let moment = Ptr(state.moment as *mut Parameters);
    let velocity = Ptr(state.velocity as *mut Parameters);
    let shards = Ptr(state.shards.as_mut_ptr());
    let average = state.average.map(|average| Ptr(average as *mut Parameters));
    let ema = options.ema;
    let mut current = Step {
        rows: Vec::new(),
        shard_size: 1,
        update: AdamUpdate {
            step: 0,
            rate,
            l2: options.l2,
        },
        finished: false,
    };
    let current_ptr = Ptr(&mut current as *mut Step<'a>);
    let mut losses = vec![0.0_f64; shard_count];
    let losses_ptr = Ptr(losses.as_mut_ptr());
    let input_len = INPUT_FEATURES * HIDDEN_SIZE;
    let range = input_len.div_ceil(shard_count);
    let k = options.k;

    thread::scope(|scope| {
        for worker in 0..shard_count {
            let barrier = &barrier;
            scope.spawn(move || {
                loop {
                    barrier.wait();
                    // SAFETY: the main thread wrote `current` before this barrier
                    // and does not touch it until the third one.
                    let step = unsafe { &*current_ptr.get() };
                    if step.finished {
                        break;
                    }
                    let rows = step.rows.chunks(step.shard_size).nth(worker).unwrap_or(&[]);
                    // SAFETY: phase two reads the parameters and writes only
                    // this worker's shard and loss slot.
                    let loss = unsafe {
                        shard_gradient(
                            &*parameters.get(),
                            rows,
                            step.rows.len(),
                            k,
                            &mut *shards.get().add(worker),
                        )
                    };
                    unsafe { *losses_ptr.get().add(worker) = loss };
                    barrier.wait();
                    // SAFETY: phase three reads every shard and writes only
                    // this worker's range of the parameters and the average
                    // (and, for worker zero, the small tensors nobody else
                    // writes).
                    unsafe {
                        let used = std::slice::from_raw_parts(shards.get(), shard_count);
                        let parameters = &mut *parameters.get();
                        let moment = &mut *moment.get();
                        let velocity = &mut *velocity.get();
                        let lo = (worker * range).min(input_len);
                        let hi = ((worker + 1) * range).min(input_len);
                        adam(
                            &mut parameters.input[lo..hi],
                            &mut moment.input[lo..hi],
                            &mut velocity.input[lo..hi],
                            |index| used.iter().map(|shard| shard.input[lo + index]).sum(),
                            step.update,
                            INPUT_LIMIT,
                        );
                        if worker == 0 {
                            adam(
                                &mut parameters.hidden,
                                &mut moment.hidden,
                                &mut velocity.hidden,
                                |index| used.iter().map(|shard| shard.hidden[index]).sum(),
                                step.update,
                                INPUT_LIMIT,
                            );
                            adam(
                                &mut parameters.output,
                                &mut moment.output,
                                &mut velocity.output,
                                |index| used.iter().map(|shard| shard.output[index]).sum(),
                                step.update,
                                OUTPUT_LIMIT,
                            );
                            adam(
                                &mut parameters.bias,
                                &mut moment.bias,
                                &mut velocity.bias,
                                |index| used.iter().map(|shard| shard.bias[index]).sum(),
                                step.update,
                                BIAS_LIMIT,
                            );
                        }
                        if let Some((average, decay)) = average.zip(ema) {
                            let average = &mut *average.get();
                            blend(&mut average.input[lo..hi], &parameters.input[lo..hi], decay);
                            if worker == 0 {
                                blend(&mut average.hidden, &parameters.hidden, decay);
                                blend(&mut average.output, &parameters.output, decay);
                                blend(&mut average.bias, &parameters.bias, decay);
                            }
                        }
                    }
                    barrier.wait();
                }
            });
        }

        let mut loss_sum = 0.0_f64;
        let mut batches = 0_usize;
        let mut seconds = [0.0_f64; 2];
        for batch in order.chunks(options.batch_size) {
            *step += 1;
            batches += 1;
            // SAFETY: no worker reads `current` until the barrier below.
            unsafe {
                let current = &mut *current_ptr.get();
                current.rows.clear();
                current
                    .rows
                    .extend(batch.iter().map(|&index| &training[index]));
                current.shard_size = batch.len().div_ceil(shard_count).max(1);
                current.update.step = *step;
            }
            let phase = Instant::now();
            barrier.wait();
            barrier.wait();
            seconds[0] += phase.elapsed().as_secs_f64();
            // SAFETY: every worker wrote its loss before the second barrier.
            let squared_errors =
                unsafe { std::slice::from_raw_parts(losses_ptr.get(), shard_count) }
                    .iter()
                    .take(
                        batch
                            .len()
                            .div_ceil(batch.len().div_ceil(shard_count).max(1)),
                    )
                    .sum::<f64>();
            let loss = squared_errors / batch.len() as f64;
            if !loss.is_finite() {
                // Let the workers finish the step before unwinding.
                barrier.wait();
                unsafe { (*current_ptr.get()).finished = true };
                barrier.wait();
                return Err("nonfinite training loss".to_owned());
            }
            loss_sum += loss;
            let phase = Instant::now();
            barrier.wait();
            seconds[1] += phase.elapsed().as_secs_f64();
        }
        // SAFETY: workers are parked on the first barrier of the next step.
        unsafe { (*current_ptr.get()).finished = true };
        barrier.wait();
        Ok((loss_sum, batches, seconds))
    })
}

struct Epoch {
    epoch: usize,
    steps: usize,
    training_loss: f64,
    /// Wall seconds spent in gradient shards, Adam reduction and metrics.
    seconds: [f64; 3],
    training: Metrics,
    development: Metrics,
    /// The moving average's training and development metrics, when kept.
    average: Option<[Metrics; 2]>,
}

impl Epoch {
    fn json(&self) -> String {
        let average = self
            .average
            .map_or_else(String::new, |[training, development]| {
                format!(
                    "\"average\": {{\"integer_development\": {}, \"integer_training\": {}}}, ",
                    development.json(),
                    training.json()
                )
            });
        format!(
            "{{{average}\"epoch\": {}, \"float_training_loss\": {}, \"integer_development\": {}, \"integer_training\": {}, \"seconds\": {{\"gradients\": {:.3}, \"adam\": {:.3}, \"metrics\": {:.3}}}, \"steps\": {}}}",
            self.epoch,
            self.training_loss,
            self.development.json(),
            self.training.json(),
            self.seconds[0],
            self.seconds[1],
            self.seconds[2],
            self.steps
        )
    }
}

/// Trains from a prepared dataset directory and writes the selected network.
///
/// Every epoch is exported and scored with integer inference. The published
/// epoch must improve the training label loss over initialization and has the
/// lowest development label loss among those; the history records all. With
/// a moving average, both weight sets are scored every epoch under the same
/// rule: the best averaged epoch is published and the best raw one is kept
/// as `raw-network.nnue`.
pub fn train(options: &Options, data: &Path, output: &Path) -> Result<String, String> {
    if output.exists() {
        return Err("output already exists; choose a new directory".to_owned());
    }
    let started = Instant::now();
    let input_hashes = prepare::verify_dataset(data)?;
    let verified = started.elapsed().as_secs_f64();
    let Split {
        rows: training,
        metrics: training_sample,
    } = read_split(
        &data.join("training.tsv"),
        BLOCK_BYTES,
        options.label_mix,
        options.k,
        options.threads,
        Keep::Training {
            metric_rows: TRAIN_METRIC_ROWS,
        },
    )?;
    let development = read_split(
        &data.join("development.tsv"),
        BLOCK_BYTES,
        options.label_mix,
        options.k,
        options.threads,
        Keep::Metrics,
    )?
    .metrics;
    let mut support = vec![false; INPUT_FEATURES];
    for row in &training {
        for side in &row.features {
            for &feature in &side[..usize::from(row.count)] {
                support[usize::from(feature)] = true;
            }
        }
    }
    let initial_bytes = options
        .init_network
        .as_ref()
        .map(|path| fs::read(path).map_err(|error| format!("{}: {error}", path.to_string_lossy())))
        .transpose()?;
    let mut parameters = match &initial_bytes {
        Some(bytes) => {
            let parameters = Parameters::dequantize(bytes)?;
            parameters.extend_support(&mut support);
            parameters
        }
        None => Parameters::initialize(&support, options.seed),
    };
    let mut moment = Parameters::zeros();
    let mut velocity = Parameters::zeros();
    let mut shards = (0..SHARDS).map(|_| Parameters::zeros()).collect::<Vec<_>>();
    // The average starts at the initial weights, so it needs no bias
    // correction; a random start is forgotten within the first epoch.
    let mut average = options.ema.map(|_| parameters.clone());
    let initial_network =
        Network::from_bytes(&parameters.quantize(&support)?).map_err(|error| error.to_string())?;
    eprintln!(
        "nnue-data: verified inputs in {verified:.1} s, loaded {} rows in {:.1} s",
        training.len(),
        started.elapsed().as_secs_f64() - verified
    );
    let initial_training = integer_metrics(
        &initial_network,
        &training_sample,
        options.k,
        options.threads,
    );
    let initial_development =
        integer_metrics(&initial_network, &development, options.k, options.threads);
    let mut random = Random(options.seed ^ 0x9E37_79B9_7F4A_7C15);
    let mut order = (0..training.len()).collect::<Vec<_>>();
    let mut step = 0;
    let mut history = Vec::new();
    let mut best: Option<(usize, Metrics, Vec<u8>)> = None;
    let mut best_average: Option<(usize, Metrics, Vec<u8>)> = None;

    let mut rate = options.rate;
    for epoch in 1..=options.epochs {
        random.shuffle(&mut order);
        let mut epoch_loss = 0.0_f64;
        let mut batches = 0_usize;
        let mut seconds = [0.0_f64; 3];
        let (loss_sum, batch_count, phase_seconds) = run_epoch(
            &training,
            &order,
            options,
            EpochState {
                parameters: &mut parameters,
                moment: &mut moment,
                velocity: &mut velocity,
                shards: &mut shards,
                average: average.as_mut(),
            },
            &mut step,
            rate,
        )?;
        epoch_loss += loss_sum;
        batches += batch_count;
        seconds[0] = phase_seconds[0];
        seconds[1] = phase_seconds[1];
        let phase = Instant::now();
        let bytes = parameters.quantize(&support)?;
        let network = Network::from_bytes(&bytes).map_err(|error| error.to_string())?;
        let training_metric =
            integer_metrics(&network, &training_sample, options.k, options.threads);
        let development_metric =
            integer_metrics(&network, &development, options.k, options.threads);
        let averaged = match &average {
            Some(average) => {
                let bytes = average.quantize(&support)?;
                let network = Network::from_bytes(&bytes).map_err(|error| error.to_string())?;
                Some((
                    integer_metrics(&network, &training_sample, options.k, options.threads),
                    integer_metrics(&network, &development, options.k, options.threads),
                    bytes,
                ))
            }
            None => None,
        };
        seconds[2] = phase.elapsed().as_secs_f64();
        let record = Epoch {
            epoch,
            steps: step,
            training_loss: epoch_loss / batches as f64,
            seconds,
            training: training_metric,
            development: development_metric,
            average: averaged
                .as_ref()
                .map(|(training, development, _)| [*training, *development]),
        };
        println!("{}", record.json());
        // A fresh run must first beat its random initialization on the
        // training sample, which rules out a diverged model; a continued run
        // starts from a network that already fits its corpus, so it must
        // instead beat that network where the epoch is selected, on the
        // development split.
        let improves_initial = |training: &Metrics, development: &Metrics| {
            if initial_bytes.is_some() {
                development.label_mse < initial_development.label_mse
            } else {
                training.label_mse < initial_training.label_mse
            }
        };
        let candidates = [(
            &mut best,
            Some((training_metric, development_metric, bytes)),
        )]
        .into_iter()
        .chain([(&mut best_average, averaged)]);
        for (best, candidate) in candidates {
            let Some((training, development, bytes)) = candidate else {
                continue;
            };
            if improves_initial(&training, &development)
                && best
                    .as_ref()
                    .is_none_or(|(_, metrics, _)| development.label_mse < metrics.label_mse)
            {
                *best = Some((epoch, development, bytes));
            }
        }
        history.push(record);
        rate *= options.rate_decay;
    }
    let (chosen, raw) = match options.ema {
        Some(_) => (best_average, best),
        None => (best, None),
    };
    let (selected, _, bytes) = chosen.ok_or(if initial_bytes.is_some() {
        "no trained integer model improved the initial network's development loss; no artifact exported"
    } else {
        "no trained integer model improved training loss; no artifact exported"
    })?;
    // Inputs are re-verified after the run so the report never binds a corpus
    // that changed underneath it.
    if prepare::verify_dataset(data)? != input_hashes {
        return Err("training inputs changed".to_owned());
    }
    let staging = output.with_extension("staging");
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(|error| error.to_string())?;
    }
    fs::create_dir_all(&staging).map_err(|error| error.to_string())?;
    fs::write(staging.join("network.nnue"), &bytes).map_err(|error| error.to_string())?;
    if let Some((_, _, raw_bytes)) = &raw {
        fs::write(staging.join("raw-network.nnue"), raw_bytes)
            .map_err(|error| error.to_string())?;
    }
    let history_json = history
        .iter()
        .map(Epoch::json)
        .collect::<Vec<_>>()
        .join(", ");
    let selected_json = history[selected - 1].json();
    let mut report = String::from("{\n");
    let _ = writeln!(report, "  \"schema_version\": 4,");
    let _ = writeln!(
        report,
        "  \"architecture\": {},",
        prepare::architecture_json()
    );
    let _ = writeln!(report, "  \"training_completed\": true,");
    let _ = writeln!(
        report,
        "  \"optimizer\": \"full-parameter Adam, float32, {SHARDS} fixed gradient shards, {} threads, SplitMix64 minibatches, weights clamped to export bounds\",",
        options.threads
    );
    let _ = writeln!(
        report,
        "  \"hyperparameters\": {{\"epochs\": {}, \"batch_size\": {}, \"rate\": {}, \"rate_decay\": {}, \"l2\": {}, \"seed\": {}, \"lambda\": {}, \"k\": {}{}}},",
        options.epochs,
        options.batch_size,
        options.rate,
        options.rate_decay,
        options.l2,
        options.seed,
        options.label_mix,
        options.k,
        options
            .ema
            .map_or_else(String::new, |decay| format!(", \"ema\": {decay}"))
    );
    let _ = writeln!(
        report,
        "  \"inputs\": {{\"data_dir\": {}, \"helper_sha256\": \"{}\", \"training_checksum\": \"{}\", \"development_checksum\": \"{}\", \"initial_network_sha256\": {}}},",
        prepare::json_string(&data.to_string_lossy()),
        prepare::helper_sha256()?,
        input_hashes["training.tsv"],
        input_hashes["development.tsv"],
        initial_bytes.as_deref().map_or_else(
            || "null".to_owned(),
            |bytes| format!("\"{}\"", sha256::hex(bytes))
        )
    );
    let _ = writeln!(report, "  \"network_sha256\": \"{}\",", sha256::hex(&bytes));
    let _ = writeln!(
        report,
        "  \"training_rows\": {}, \"development_rows\": {}, \"training_metric_rows\": {}, \"steps\": {step}, \"unsupported_feature_rows\": {},",
        training.len(),
        development.len(),
        training_sample.len(),
        support.iter().filter(|supported| !**supported).count()
    );
    let _ = writeln!(
        report,
        "  \"initial_integer_training\": {},",
        initial_training.json()
    );
    let _ = writeln!(
        report,
        "  \"initial_integer_development\": {},",
        initial_development.json()
    );
    let _ = writeln!(report, "  \"selected_epoch\": {selected},");
    let _ = writeln!(report, "  \"selected_metrics\": {selected_json},");
    if options.ema.is_some() {
        let _ = writeln!(
            report,
            "  \"raw_selected_epoch\": {}, \"raw_network_sha256\": {},",
            raw.as_ref()
                .map_or_else(|| "null".to_owned(), |(epoch, _, _)| epoch.to_string()),
            raw.as_ref().map_or_else(
                || "null".to_owned(),
                |(_, _, bytes)| format!("\"{}\"", sha256::hex(bytes))
            )
        );
    }
    let _ = writeln!(report, "  \"history\": [{history_json}],");
    let _ = writeln!(
        report,
        "  \"limitations\": [\"No Elo, speed or personality claim; external experimental model only.\", \"Development label loss selects an epoch, not an independent strength confirmation.\", \"Split-overlap rejection does not establish opening-family or game independence.\", \"Integer metrics use the engine's own network loader; the training subsample is strided to at most training_metric_rows positions.\"]"
    );
    report.push_str("}\n");
    fs::write(staging.join("report.json"), &report).map_err(|error| error.to_string())?;
    if output.exists() {
        return Err("output appeared during training".to_owned());
    }
    fs::rename(&staging, output).map_err(|error| error.to_string())?;
    Ok(format!(
        "{{\"network_sha256\": \"{}\", \"selected_epoch\": {selected}, \"steps\": {step}, \"selected_metrics\": {selected_json}}}",
        sha256::hex(&bytes)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEN: &str = "4k3/8/5n2/8/8/8/2P5/4K3 w - - 0 1";

    fn line(fen: &str, stm: u8, outcome: f32, teacher: &str) -> String {
        let board: Board = fen.parse().unwrap();
        let features = |color| {
            jakgro::engine::nnue::active_features(&board, color)
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(",")
        };
        format!(
            "{fen}\t{fen}\t{stm}\t{outcome}\t{teacher}\t{}\t{}",
            features(cozy_chess::Color::White),
            features(cozy_chess::Color::Black)
        )
    }

    fn row(fen: &str, stm: u8, outcome: f32, teacher: &str) -> Row {
        parse_row(&line(fen, stm, outcome, teacher), 0.5, 0.88)
            .unwrap()
            .0
    }

    #[test]
    fn black_labels_are_reoriented_once() {
        let (black, metric) =
            parse_row(&line(&FEN.replace(" w ", " b "), 1, 1.0, "150"), 0.5, 0.88).unwrap();
        assert_eq!(metric.outcome, 0.0);
        assert!((black.label - 0.5 * sigmoid(-150.0, 0.88)).abs() < 1e-6);
        assert_eq!(metric.label, black.label);
        let white = row(FEN, 0, 1.0, "-");
        assert_eq!(white.label, 1.0);
        assert!(parse_row(&format!("{FEN}\t{FEN}\t1\t1\t-\t0\t0"), 0.5, 0.88).is_err());
    }

    fn temporary(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "nnue-train-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn strided_rows_match_the_historical_training_subsample() {
        assert_eq!(strided(5, 8), vec![0, 1, 2, 3, 4]);
        assert_eq!(strided(40, 7), vec![0, 6, 13, 19, 26, 32, 39]);
        assert_eq!(line_count(b"a\nb\n"), 2);
        assert_eq!(line_count(b"a\nb"), 2);
        assert_eq!(line_count(b""), 0);
    }

    #[test]
    fn streamed_splits_do_not_depend_on_the_block_size() {
        let fens = [
            FEN,
            "4k3/8/5n2/8/8/8/3P4/4K3 b - - 0 1",
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r3k2r/8/8/8/8/8/8/R3K2R b KQkq - 0 1",
            "8/8/4k3/8/8/4K3/4P3/8 w - - 0 1",
        ];
        let lines = (0..40)
            .map(|index| {
                let fen = fens[index % fens.len()];
                let teacher = if index % 4 == 0 {
                    "-".to_owned()
                } else {
                    (index as i32 * 7 - 100).to_string()
                };
                let stm = u8::from(fen.contains(" b "));
                line(fen, stm, [0.0, 0.5, 1.0][index % 3], &teacher)
            })
            .collect::<Vec<_>>();
        let root = temporary("stream");
        fs::create_dir_all(&root).unwrap();
        let body = lines.join("\n");
        let terminated = root.join("terminated.tsv");
        let unterminated = root.join("unterminated.tsv");
        fs::write(&terminated, format!("{}{body}\n", *crate::HEADER)).unwrap();
        fs::write(&unterminated, format!("{}{body}", *crate::HEADER)).unwrap();
        let training = Keep::Training { metric_rows: 7 };
        let read = |path: &Path, block, keep| read_split(path, block, 0.25, 0.88, 3, keep);
        let reference = read(&terminated, 1 << 20, training).unwrap();
        assert_eq!(reference.rows.len(), 40);
        let expected = strided(40, 7)
            .into_iter()
            .map(|index| {
                fens[index % fens.len()]
                    .parse::<Board>()
                    .unwrap()
                    .to_string()
            })
            .collect::<Vec<_>>();
        let boards = |split: &Split| {
            split
                .metrics
                .iter()
                .map(|metric| metric.board.to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(boards(&reference), expected);
        for path in [&terminated, &unterminated] {
            // Six hundred bytes hold the header and one or two rows, so most
            // blocks end mid-file and carry a partial line into the next.
            for block in [600, 1 << 20] {
                let split = read(path, block, training).unwrap();
                assert_eq!(boards(&split), expected);
                assert_eq!(split.rows.len(), reference.rows.len());
                for (row, expected) in split.rows.iter().zip(&reference.rows) {
                    assert_eq!(row.features, expected.features);
                    assert_eq!((row.count, row.bucket), (expected.count, expected.bucket));
                    assert_eq!(row.label.to_bits(), expected.label.to_bits());
                }
                let development = read(path, block, Keep::Metrics).unwrap();
                assert!(development.rows.is_empty());
                assert_eq!(development.metrics.len(), 40);
                assert_eq!(
                    development.metrics[3].board.to_string(),
                    fens[3].parse::<Board>().unwrap().to_string()
                );
            }
        }
        let error = read(&terminated, 100, training).err().unwrap();
        assert!(error.contains("exceeds"), "{error}");
        fs::write(&terminated, crate::HEADER.as_bytes()).unwrap();
        let error = read(&terminated, 1 << 20, training).err().unwrap();
        assert!(error.contains("empty dataset"), "{error}");
        fs::write(
            &terminated,
            format!("{}{}\n\n{}\n", *crate::HEADER, lines[0], lines[1]),
        )
        .unwrap();
        let error = read(&terminated, 1 << 20, training).err().unwrap();
        assert!(error.contains(":4:"), "{error}");
        fs::remove_dir_all(&root).unwrap();
    }

    /// A prepared dataset of twelve training positions and one development
    /// position in a fresh directory, under `data`.
    fn prepared(name: &str) -> PathBuf {
        let root = temporary(name);
        fs::create_dir_all(&root).unwrap();
        let mut training = String::new();
        for (index, knight) in ["5n2", "2n5"].iter().enumerate() {
            for file in 1..=6 {
                let _ = writeln!(
                    training,
                    "4k3/8/{knight}/8/8/8/{file}P{}/4K3 w - - 0 1;{};{}",
                    7 - file,
                    [0.0, 0.5, 1.0][(file + index) % 3],
                    file as i32 * 40 - 150
                );
            }
        }
        fs::write(root.join("training.txt"), training).unwrap();
        fs::write(
            root.join("development.txt"),
            "4k3/8/8/3n4/8/8/1P6/4K3 w - - 0 1;1;30\n",
        )
        .unwrap();
        prepare::prepare(&prepare::Options {
            training: root.join("training.txt"),
            development: root.join("development.txt"),
            output: root.join("data"),
            deduplicate: false,
            drop_overlap: false,
        })
        .unwrap();
        root
    }

    fn small_run() -> Options {
        Options {
            epochs: 3,
            batch_size: 4,
            rate: 0.01,
            rate_decay: 1.0,
            l2: 0.0,
            seed: 5,
            label_mix: 0.25,
            k: 0.88,
            threads: 2,
            init_network: None,
            ema: None,
        }
    }

    #[test]
    fn a_moving_average_leaves_the_raw_trajectory_unchanged() {
        let root = prepared("ema");
        let data = root.join("data");
        train(&small_run(), &data, &root.join("plain")).unwrap();
        let averaged = Options {
            ema: Some(0.5),
            ..small_run()
        };
        train(&averaged, &data, &root.join("averaged")).unwrap();
        let read = |name: &str| fs::read(root.join(name)).unwrap();
        assert_eq!(
            read("plain/network.nnue"),
            read("averaged/raw-network.nnue")
        );
        assert_ne!(
            read("averaged/network.nnue"),
            read("averaged/raw-network.nnue")
        );
        assert!(!root.join("plain/raw-network.nnue").exists());
        let report = fs::read_to_string(root.join("averaged/report.json")).unwrap();
        for key in [
            "\"ema\": 0.5",
            "\"average\": {",
            "\"raw_network_sha256\": \"",
        ] {
            assert!(report.contains(key), "{key}");
        }
        let plain = fs::read_to_string(root.join("plain/report.json")).unwrap();
        for key in ["\"ema\"", "\"average\"", "raw_"] {
            assert!(!plain.contains(key), "{key}");
        }
        assert!(
            Options::parse(
                &["--data-dir", "d", "--output-dir", "o", "--ema", "1"].map(String::from)
            )
            .is_err()
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn analytic_gradients_match_finite_differences() {
        let rows = [
            row(FEN, 0, 1.0, "40"),
            row(&FEN.replace(" w ", " b "), 1, 0.0, "-80"),
        ];
        let refs = rows.iter().collect::<Vec<_>>();
        let support = vec![true; INPUT_FEATURES];
        let mut parameters = Parameters::initialize(&support, 3);
        parameters.hidden.fill(0.2);
        let mut gradient = Parameters::zeros();
        shard_gradient(&parameters, &refs, refs.len(), 0.88, &mut gradient);
        let loss = |parameters: &Parameters| -> f64 {
            refs.iter()
                .map(|row| {
                    let (_, raw) = parameters.forward(row);
                    let prediction = f64::from(sigmoid(raw, 0.88));
                    (prediction - f64::from(row.label)).powi(2)
                })
                .sum::<f64>()
                / refs.len() as f64
        };
        let feature = usize::from(rows[0].features[0][1]) * HIDDEN_SIZE + 3;
        let bucket = usize::from(rows[0].bucket);
        for (name, index) in [
            ("input", feature),
            ("hidden", 3),
            ("output", (bucket * 2 + 1) * HIDDEN_SIZE + 9),
            ("bias", bucket),
        ] {
            fn select<'p>(probe: &'p mut Parameters, name: &str, index: usize) -> &'p mut f32 {
                match name {
                    "input" => &mut probe.input[index],
                    "hidden" => &mut probe.hidden[index],
                    "output" => &mut probe.output[index],
                    _ => &mut probe.bias[index],
                }
            }
            let mut probe = parameters.clone();
            let epsilon = 1e-3_f32;
            *select(&mut probe, name, index) += epsilon;
            let plus = loss(&probe);
            *select(&mut probe, name, index) -= 2.0 * epsilon;
            let minus = loss(&probe);
            let numerical = (plus - minus) / (2.0 * f64::from(epsilon));
            let analytic = f64::from(match name {
                "input" => gradient.input[index],
                "hidden" => gradient.hidden[index],
                "output" => gradient.output[index],
                _ => gradient.bias[index],
            });
            assert!(
                (numerical - analytic).abs() < 1e-3 * (1.0 + analytic.abs()),
                "{name}[{index}]: numerical {numerical} analytic {analytic}"
            );
        }
    }

    #[test]
    fn exported_bytes_reload_and_unsupported_rows_stay_zero() {
        let mut support = vec![false; INPUT_FEATURES];
        support[10] = true;
        let parameters = Parameters::initialize(&support, 9);
        let network = Network::from_bytes(&parameters.quantize(&support).unwrap()).unwrap();
        let board: Board = FEN.parse().unwrap();
        let accumulator = network.accumulator(&board);
        // Unsupported rows contribute nothing: only the hidden bias is left.
        assert!(
            accumulator
                .values(cozy_chess::Color::White)
                .iter()
                .all(|&value| value == (0.1_f32 * 255.0).round() as i32)
        );
        let mut leaked = parameters.clone();
        leaked.input[5 * HIDDEN_SIZE] = 0.5;
        assert!(leaked.quantize(&support).is_err());
    }

    #[test]
    fn dequantized_parameters_export_the_same_network_and_extend_support() {
        let mut support = vec![false; INPUT_FEATURES];
        support[10] = true;
        support[4000] = true;
        let mut parameters = Parameters::initialize(&support, 9);
        parameters.output[3] = 0.75;
        parameters.bias[2] = 0.03;
        let bytes = parameters.quantize(&support).unwrap();

        let recovered = Parameters::dequantize(&bytes).unwrap();
        // Quantizing the recovered parameters reproduces the file exactly:
        // every recovered value sits within half a step of the exported one.
        assert_eq!(recovered.quantize(&support).unwrap(), bytes);
        for (recovered, original) in recovered.hidden.iter().zip(&parameters.hidden) {
            assert!((recovered - original).abs() <= 0.5 / ACTIVATION_MAX as f32);
        }
        // The rows the network carries join the support of a corpus that
        // never shows their features, so the export keeps what was learned.
        let mut narrow = vec![false; INPUT_FEATURES];
        recovered.extend_support(&mut narrow);
        assert!(narrow[10] && narrow[4000]);
        assert_eq!(narrow.iter().filter(|supported| **supported).count(), 2);
        assert!(recovered.quantize(&narrow).is_ok());

        assert!(Parameters::dequantize(&bytes[..bytes.len() - 1]).is_err());
    }
}
