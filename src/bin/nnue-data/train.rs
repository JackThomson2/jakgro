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
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::thread;

use cozy_chess::Board;
use jakgro::engine::nnue::{
    ACTIVATION_MAX, FILE_BYTES, FORMAT_VERSION, HEADER_BYTES, HIDDEN_SIZE, INPUT_FEATURES, MAGIC,
    MAX_SCORE, Network, OUTPUT_SCALE,
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

const HEADER: &str = "# jakgro-nnue-data-v1\t12288\t128\t255\t64";
const COLUMNS: &str = "fen\tkey\tstm\toutcome\tteacher_cp\twhite\tblack";

#[derive(Clone, Debug)]
pub struct Options {
    pub epochs: usize,
    pub batch_size: usize,
    pub rate: f32,
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

fn read_rows(path: &Path, label_mix: f32, k: f32) -> Result<Vec<Row>, String> {
    let file = fs::File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut lines = BufReader::new(file).lines();
    let mut header = || {
        lines
            .next()
            .transpose()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("{}: truncated header", path.display()))
    };
    if header()? != HEADER {
        return Err(format!("{}: unsupported feature schema", path.display()));
    }
    if header()? != COLUMNS {
        return Err(format!("{}: wrong dataset columns", path.display()));
    }
    let mut rows = Vec::new();
    for (index, line) in lines.enumerate() {
        let line = line.map_err(|error| error.to_string())?;
        let row = parse_row(&line, label_mix, k)
            .map_err(|error| format!("{}:{}: {error}", path.display(), index + 3))?;
        rows.push(row);
    }
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
    output: Vec<f32>,
    bias: f32,
}

impl Parameters {
    fn zeros() -> Self {
        Self {
            input: vec![0.0; INPUT_FEATURES * HIDDEN_SIZE],
            hidden: vec![0.0; HIDDEN_SIZE],
            output: vec![0.0; 2 * HIDDEN_SIZE],
            bias: 0.0,
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
    fn forward(&self, row: &Row) -> ([[f32; HIDDEN_SIZE]; 2], f32) {
        let mut sums = [[0.0_f32; HIDDEN_SIZE]; 2];
        let mut total = self.bias;
        for (side, sum) in sums.iter_mut().enumerate() {
            sum.copy_from_slice(&self.hidden);
            for &feature in &row.features[side][..usize::from(row.count)] {
                let start = usize::from(feature) * HIDDEN_SIZE;
                for (value, &weight) in sum.iter_mut().zip(&self.input[start..start + HIDDEN_SIZE])
                {
                    *value += weight;
                }
            }
            let weights = &self.output[side * HIDDEN_SIZE..(side + 1) * HIDDEN_SIZE];
            for (&value, &weight) in sum.iter().zip(weights) {
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
        let bias =
            (f64::from(self.bias) * f64::from(FLOAT_CP_SCALE) * f64::from(CP_DIVISOR)).round();
        if !bias.is_finite() || bias < f64::from(i32::MIN) || bias > f64::from(i32::MAX) {
            return Err("output bias exceeds storage bounds".to_owned());
        }
        payload.extend_from_slice(&(bias as i32).to_le_bytes());
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
            0,
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
/// Returns the shard's sum of squared errors.
fn shard_gradient(
    parameters: &Parameters,
    rows: &[&Row],
    batch: usize,
    k: f32,
    gradient: &mut Parameters,
) -> f64 {
    gradient.input.fill(0.0);
    gradient.hidden.fill(0.0);
    gradient.output.fill(0.0);
    gradient.bias = 0.0;
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
        gradient.bias += outer;
        for (side, sums) in sums.iter().enumerate() {
            let weights = &parameters.output[side * HIDDEN_SIZE..(side + 1) * HIDDEN_SIZE];
            let mut hidden_gradient = [0.0_f32; HIDDEN_SIZE];
            for (unit, &sum) in sums.iter().enumerate() {
                gradient.output[side * HIDDEN_SIZE + unit] += outer * sum.clamp(0.0, 1.0);
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
    step: usize,
    rate: f32,
    l2: f32,
    limit: f32,
) {
    let moment_correction = 1.0 - 0.9_f32.powi(step as i32);
    let velocity_correction = 1.0 - 0.999_f32.powi(step as i32);
    for index in 0..parameters.len() {
        let g = gradient(index) + l2 * parameters[index];
        moment[index] = 0.9 * moment[index] + 0.1 * g;
        velocity[index] = 0.999 * velocity[index] + 0.001 * g * g;
        parameters[index] = (parameters[index]
            - rate * (moment[index] / moment_correction)
                / ((velocity[index] / velocity_correction).sqrt() + 1e-8))
            .clamp(-limit, limit);
    }
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

struct Epoch {
    epoch: usize,
    steps: usize,
    training_loss: f64,
    training: Metrics,
    development: Metrics,
}

impl Epoch {
    fn json(&self) -> String {
        format!(
            "{{\"epoch\": {}, \"float_training_loss\": {}, \"integer_development\": {}, \"integer_training\": {}, \"steps\": {}}}",
            self.epoch,
            self.training_loss,
            self.development.json(),
            self.training.json(),
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
    let training = read_rows(&data.join("training.tsv"), options.label_mix, options.k)?;
    let development = read_rows(&data.join("development.tsv"), options.label_mix, options.k)?;
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
    let rows_per_thread = |len: usize| len.div_ceil(options.threads).max(1);

    for epoch in 1..=options.epochs {
        random.shuffle(&mut order);
        let mut epoch_loss = 0.0_f64;
        let mut batches = 0_usize;
        for batch in order.chunks(options.batch_size) {
            step += 1;
            batches += 1;
            let rows = batch
                .iter()
                .map(|&index| &training[index])
                .collect::<Vec<_>>();
            let shard_size = rows.len().div_ceil(SHARDS).max(1);
            // Phase one: every shard accumulates its slice's batch-mean gradient.
            let squared_errors = thread::scope(|scope| {
                let parameters = &parameters;
                let handles = shards
                    .iter_mut()
                    .zip(rows.chunks(shard_size))
                    .map(|(shard, rows)| {
                        scope.spawn(move || {
                            shard_gradient(parameters, rows, batch.len(), options.k, shard)
                        })
                    })
                    .collect::<Vec<_>>();
                handles
                    .into_iter()
                    .map(|handle| handle.join().expect("gradient worker panicked"))
                    .sum::<f64>()
            });
            let loss = squared_errors / batch.len() as f64;
            if !loss.is_finite() {
                return Err("nonfinite training loss".to_owned());
            }
            epoch_loss += loss;
            let used = &shards[..rows.chunks(shard_size).len()];
            // Phase two: fixed parameter ranges reduce the shards in order.
            let chunk = rows_per_thread(parameters.input.len());
            thread::scope(|scope| {
                let ranges = parameters
                    .input
                    .chunks_mut(chunk)
                    .zip(moment.input.chunks_mut(chunk))
                    .zip(velocity.input.chunks_mut(chunk));
                for (index, ((values, moment), velocity)) in ranges.enumerate() {
                    let offset = index * chunk;
                    scope.spawn(move || {
                        adam(
                            values,
                            moment,
                            velocity,
                            |index| used.iter().map(|shard| shard.input[offset + index]).sum(),
                            step,
                            options.rate,
                            options.l2,
                            INPUT_LIMIT,
                        );
                    });
                }
            });
            adam(
                &mut parameters.hidden,
                &mut moment.hidden,
                &mut velocity.hidden,
                |index| used.iter().map(|shard| shard.hidden[index]).sum(),
                step,
                options.rate,
                options.l2,
                INPUT_LIMIT,
            );
            adam(
                &mut parameters.output,
                &mut moment.output,
                &mut velocity.output,
                |index| used.iter().map(|shard| shard.output[index]).sum(),
                step,
                options.rate,
                options.l2,
                OUTPUT_LIMIT,
            );
            let bias_gradient = used.iter().map(|shard| shard.bias).sum::<f32>();
            adam(
                std::slice::from_mut(&mut parameters.bias),
                std::slice::from_mut(&mut moment.bias),
                std::slice::from_mut(&mut velocity.bias),
                |_| bias_gradient,
                step,
                options.rate,
                options.l2,
                BIAS_LIMIT,
            );
        }
        let bytes = parameters.quantize(&support)?;
        let network = Network::from_bytes(&bytes).map_err(|error| error.to_string())?;
        let training_metric =
            integer_metrics(&network, &training_sample, options.k, options.threads);
        let development_metric =
            integer_metrics(&network, &development, options.k, options.threads);
        let record = Epoch {
            epoch,
            steps: step,
            training_loss: epoch_loss / batches as f64,
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
    }
    let (selected, _, bytes) =
        best.ok_or("no trained integer model improved training loss; no artifact exported")?;
    let staging = output.with_extension("staging");
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(|error| error.to_string())?;
    }
    fs::create_dir_all(&staging).map_err(|error| error.to_string())?;
    fs::write(staging.join("network.nnue"), &bytes).map_err(|error| error.to_string())?;
    let mut summary = String::new();
    let _ = write!(
        summary,
        "{{\"initial_integer_development\": {}, \"initial_integer_training\": {}, \"history\": [",
        initial_development.json(),
        initial_training.json()
    );
    for (index, record) in history.iter().enumerate() {
        if index > 0 {
            summary.push_str(", ");
        }
        summary.push_str(&record.json());
    }
    let _ = write!(
        summary,
        "], \"selected_epoch\": {selected}, \"steps\": {step}, \"training_rows\": {}, \"development_rows\": {}, \"training_metric_rows\": {}, \"threads\": {}, \"shards\": {SHARDS}, \"unsupported_feature_rows\": {}}}",
        training.len(),
        development.len(),
        training_sample.len(),
        options.threads,
        support.iter().filter(|supported| !**supported).count()
    );
    fs::write(staging.join("training.json"), format!("{summary}\n"))
        .map_err(|error| error.to_string())?;
    if output.exists() {
        return Err("output appeared during training".to_owned());
    }
    fs::rename(&staging, output).map_err(|error| error.to_string())?;
    Ok(summary)
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
        for (name, index) in [
            ("input", feature),
            ("hidden", 3),
            ("output", HIDDEN_SIZE + 9),
            ("bias", 0),
        ] {
            fn select<'p>(probe: &'p mut Parameters, name: &str, index: usize) -> &'p mut f32 {
                match name {
                    "input" => &mut probe.input[index],
                    "hidden" => &mut probe.hidden[index],
                    "output" => &mut probe.output[index],
                    _ => &mut probe.bias,
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
                _ => gradient.bias,
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
