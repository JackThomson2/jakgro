//! Multi-threaded CPU trainer for the fixed Jakgro NNUE architecture.
//!
//! The float model mirrors the Python reference exactly: unclipped sums are
//! `hidden + Σ input[feature]`, activations clip to `0..=1`, the score is
//! `400 * (Σ output · activation + bias)` centipawns, and the loss is the mean
//! squared error between the sigmoid of that score and a label mixing the game
//! outcome with the teacher's own sigmoid. Full-parameter Adam runs over
//! seeded minibatches; the exported integers are the same quantization the
//! Python exporter used, and every published epoch is re-read through the
//! engine's own [`Network`] loader for its integer metrics.
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
    ACTIVATION_MAX, FILE_BYTES, FORMAT_VERSION, HEADER_BYTES, HIDDEN_SIZE, INPUT_FEATURES, MAGIC,
    MAX_SCORE, Network, OUTPUT_BUCKETS, OUTPUT_SCALE, active_features,
};

/// Centipawns per unit of float output, shared with the Python reference.
const FLOAT_CP_SCALE: f32 = 400.0;
/// Integer output divisor, `ACTIVATION_MAX * OUTPUT_SCALE`.
const CP_DIVISOR: i32 = ACTIVATION_MAX * OUTPUT_SCALE;
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
        check((1..=256).contains(&self.threads), "threads must be 1..256")
    }
}

/// One prepared position, oriented to the side to move.
#[derive(Clone)]
struct Row {
    board: Board,
    /// Side-to-move perspective features, then the opponent's.
    features: [[u16; MAX_PIECES]; 2],
    count: u8,
    /// Output layer selected by the piece count.
    bucket: u8,
    /// Side-to-move outcome in points.
    outcome: f32,
    /// Fitted label after mixing outcome and teacher probability.
    label: f32,
}

fn sigmoid_scale(k: f32) -> f32 {
    std::f32::consts::LN_10 * k / 400.0
}

fn sigmoid(score: f32, k: f32) -> f32 {
    1.0 / (1.0 + (-(score * sigmoid_scale(k)).clamp(-80.0, 80.0)).exp())
}

/// Loads a prepared split, parsing line chunks on every thread.
///
/// Every row's stored features are recomputed from its FEN with the engine's
/// own mapping, so a dataset prepared by another feature set is rejected here
/// rather than trusted.
fn read_rows(path: &Path, label_mix: f32, k: f32, threads: usize) -> Result<Vec<Row>, String> {
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut offset = 0;
    for expected in crate::HEADER.lines() {
        let end = bytes[offset..]
            .iter()
            .position(|&byte| byte == b'\n')
            .map(|end| offset + end)
            .ok_or_else(|| format!("{}: truncated header", path.display()))?;
        if &bytes[offset..end] != expected.as_bytes() {
            let what = if offset == 0 {
                "unsupported feature schema"
            } else {
                "wrong dataset columns"
            };
            return Err(format!("{}: {what}", path.display()));
        }
        offset = end + 1;
    }
    let body = &bytes[offset..];
    // Chunk boundaries fall on newlines so every line is parsed exactly once.
    let target = body.len().div_ceil(threads * 4).max(1);
    let mut bounds = vec![0];
    while *bounds.last().unwrap() < body.len() {
        let start = *bounds.last().unwrap();
        let end = (start + target).min(body.len());
        let end = body[end..]
            .iter()
            .position(|&byte| byte == b'\n')
            .map_or(body.len(), |extra| end + extra + 1);
        bounds.push(end);
    }
    let lines_before: Vec<usize> = {
        let mut counts = vec![0];
        for pair in bounds.windows(2) {
            let last = *counts.last().unwrap();
            counts.push(
                last + body[pair[0]..pair[1]]
                    .iter()
                    .filter(|&&byte| byte == b'\n')
                    .count(),
            );
        }
        counts
    };
    let chunks = thread::scope(|scope| {
        let handles = bounds
            .windows(2)
            .zip(&lines_before)
            .map(|(pair, &first_line)| {
                let chunk = &body[pair[0]..pair[1]];
                scope.spawn(move || -> Result<Vec<Row>, String> {
                    let text = std::str::from_utf8(chunk)
                        .map_err(|_| format!("{}: invalid UTF-8", path.display()))?;
                    let mut rows = Vec::with_capacity(chunk.len() / 200);
                    for (index, line) in text.lines().enumerate() {
                        let row = parse_row(line, label_mix, k).map_err(|error| {
                            format!("{}:{}: {error}", path.display(), first_line + index + 3)
                        })?;
                        rows.push(row);
                    }
                    Ok(rows)
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("row parser panicked"))
            .collect::<Result<Vec<_>, _>>()
    })?;
    let rows = chunks.into_iter().flatten().collect::<Vec<_>>();
    if rows.is_empty() {
        return Err(format!("{}: empty dataset", path.display()));
    }
    Ok(rows)
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

fn parse_row(line: &str, label_mix: f32, k: f32) -> Result<Row, String> {
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
    Ok(Row {
        board,
        features: if stm == 0 {
            [white, black]
        } else {
            [black, white]
        },
        count: white_count as u8,
        bucket: ((white_count - 1) / 4) as u8,
        outcome,
        label,
    })
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
                total += value.clamp(0.0, 1.0) * weight;
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
            payload.extend_from_slice(
                &scale_i16(value, FLOAT_CP_SCALE * OUTPUT_SCALE as f32)?.to_le_bytes(),
            );
        }
        for &value in &self.bias {
            let bias =
                (f64::from(value) * f64::from(FLOAT_CP_SCALE) * f64::from(CP_DIVISOR)).round();
            if !bias.is_finite() || bias < f64::from(i32::MIN) || bias > f64::from(i32::MAX) {
                return Err("output bias exceeds storage bounds".to_owned());
            }
            payload.extend_from_slice(&(bias as i32).to_le_bytes());
        }
        let mut bytes = Vec::with_capacity(FILE_BYTES);
        bytes.extend_from_slice(&MAGIC);
        for field in [
            FORMAT_VERSION,
            1,
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
                gradient.output[start + unit] += outer * sum.clamp(0.0, 1.0);
                if sum > 0.0 && sum < 1.0 {
                    hidden_gradient[unit] = outer * weights[unit];
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

/// The step-dependent Adam settings shared by every tensor in one update.
#[derive(Clone, Copy)]
struct AdamUpdate {
    step: usize,
    rate: f32,
    l2: f32,
}

/// Largest float magnitudes the i16/i32 export can represent per tensor.
const INPUT_LIMIT: f32 = i16::MAX as f32 / ACTIVATION_MAX as f32;
const OUTPUT_LIMIT: f32 = i16::MAX as f32 / (FLOAT_CP_SCALE * OUTPUT_SCALE as f32);
const BIAS_LIMIT: f32 = i32::MAX as f32 / (FLOAT_CP_SCALE * CP_DIVISOR as f32);

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
fn integer_metrics(network: &Network, rows: &[Row], k: f32, threads: usize) -> Metrics {
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

fn subsample(rows: &[Row], maximum: usize) -> Vec<Row> {
    if rows.len() <= maximum {
        return rows.to_vec();
    }
    (0..maximum)
        .map(|index| rows[index * (rows.len() - 1) / (maximum - 1)].clone())
        .collect()
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
/// while reading all shards. Worker zero also owns the small tensors. Returns
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
                    // this worker's parameter range (and, for worker zero, the
                    // small tensors nobody else writes).
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
}

impl Epoch {
    fn json(&self) -> String {
        format!(
            "{{\"epoch\": {}, \"float_training_loss\": {}, \"integer_development\": {}, \"integer_training\": {}, \"seconds\": {{\"gradients\": {:.3}, \"adam\": {:.3}, \"metrics\": {:.3}}}, \"steps\": {}}}",
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
/// lowest development label loss among those; the history records all.
pub fn train(options: &Options, data: &Path, output: &Path) -> Result<String, String> {
    if output.exists() {
        return Err("output already exists; choose a new directory".to_owned());
    }
    let started = Instant::now();
    let input_hashes = prepare::verify_dataset(data)?;
    let verified = started.elapsed().as_secs_f64();
    let training = read_rows(
        &data.join("training.tsv"),
        options.label_mix,
        options.k,
        options.threads,
    )?;
    let development = read_rows(
        &data.join("development.tsv"),
        options.label_mix,
        options.k,
        options.threads,
    )?;
    let mut support = vec![false; INPUT_FEATURES];
    for row in &training {
        for side in &row.features {
            for &feature in &side[..usize::from(row.count)] {
                support[usize::from(feature)] = true;
            }
        }
    }
    let training_sample = subsample(&training, TRAIN_METRIC_ROWS);
    let mut parameters = Parameters::initialize(&support, options.seed);
    let mut moment = Parameters::zeros();
    let mut velocity = Parameters::zeros();
    let mut shards = (0..SHARDS).map(|_| Parameters::zeros()).collect::<Vec<_>>();
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
        seconds[2] = phase.elapsed().as_secs_f64();
        let record = Epoch {
            epoch,
            steps: step,
            training_loss: epoch_loss / batches as f64,
            seconds,
            training: training_metric,
            development: development_metric,
        };
        println!("{}", record.json());
        if training_metric.label_mse < initial_training.label_mse
            && best
                .as_ref()
                .is_none_or(|(_, metrics, _)| development_metric.label_mse < metrics.label_mse)
        {
            best = Some((epoch, development_metric, bytes));
        }
        history.push(record);
        rate *= options.rate_decay;
    }
    let (selected, _, bytes) =
        best.ok_or("no trained integer model improved training loss; no artifact exported")?;
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
    let history_json = history
        .iter()
        .map(Epoch::json)
        .collect::<Vec<_>>()
        .join(", ");
    let selected_json = history[selected - 1].json();
    let mut report = String::from("{\n");
    let _ = writeln!(report, "  \"schema_version\": 3,");
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
        "  \"hyperparameters\": {{\"epochs\": {}, \"batch_size\": {}, \"rate\": {}, \"rate_decay\": {}, \"l2\": {}, \"seed\": {}, \"lambda\": {}, \"k\": {}}},",
        options.epochs,
        options.batch_size,
        options.rate,
        options.rate_decay,
        options.l2,
        options.seed,
        options.label_mix,
        options.k
    );
    let _ = writeln!(
        report,
        "  \"inputs\": {{\"data_dir\": {}, \"helper_sha256\": \"{}\", \"training_checksum\": \"{}\", \"development_checksum\": \"{}\"}},",
        prepare::json_string(&data.to_string_lossy()),
        prepare::helper_sha256()?,
        input_hashes["training.tsv"],
        input_hashes["development.tsv"]
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

    fn row(fen: &str, stm: u8, outcome: f32, teacher: &str) -> Row {
        let board: Board = fen.parse().unwrap();
        let features = |color| {
            jakgro::engine::nnue::active_features(&board, color)
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(",")
        };
        let line = format!(
            "{fen}\t{fen}\t{stm}\t{outcome}\t{teacher}\t{}\t{}",
            features(cozy_chess::Color::White),
            features(cozy_chess::Color::Black)
        );
        parse_row(&line, 0.5, 0.88).unwrap()
    }

    #[test]
    fn black_labels_are_reoriented_once() {
        let black = row(&FEN.replace(" w ", " b "), 1, 1.0, "150");
        assert_eq!(black.outcome, 0.0);
        assert!((black.label - 0.5 * sigmoid(-150.0, 0.88)).abs() < 1e-6);
        let white = row(FEN, 0, 1.0, "-");
        assert_eq!(white.label, 1.0);
        assert!(parse_row(&format!("{FEN}\t{FEN}\t1\t1\t-\t0\t0"), 0.5, 0.88).is_err());
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
}
