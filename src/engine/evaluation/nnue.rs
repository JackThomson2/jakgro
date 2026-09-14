//! Experimental, standalone quantized neural evaluation.
//!
//! This module does not select an engine backend or supply a default network.
//! Features are colored piece-square pairs, including both kings, conditioned
//! on one of sixteen 2x2 buckets of the perspective king. Black's perspective
//! mirrors ranks, not files. Both perspectives share the feature transformer.
//! The output concatenates the side-to-move accumulator before its opponent's.
//!
//! Accumulators contain unclipped integer sums. Inference clips each activation
//! to 0..=255, computes an integer dot product, and divides by 255 * 64 with
//! truncation toward zero. The result is side-to-move-relative centipawns,
//! bounded below the engine's mate region. It is not a terminal-position score.

mod format;
#[cfg(test)]
mod tests;

use std::fmt;

use cozy_chess::{BitBoard, Board, Color, Piece, Square};

pub use format::{FILE_BYTES, FORMAT_VERSION, HEADER_BYTES, LoadError, MAGIC};

/// Number of 2x2 king buckets in the oriented board.
pub const KING_BUCKETS: usize = 16;
/// Own P/N/B/R/Q/K planes followed by the opponent's P/N/B/R/Q/K planes.
pub const PIECE_PLANES: usize = 12;
/// Number of feature-transformer rows.
pub const INPUT_FEATURES: usize = KING_BUCKETS * PIECE_PLANES * 64;
/// Width of each perspective's shared feature transformer.
pub const HIDDEN_SIZE: usize = 128;
/// Integer clipped-ReLU ceiling, and feature-transformer quantization scale.
pub const ACTIVATION_MAX: i32 = 255;
/// Quantization scale for centipawn-valued output weights.
pub const OUTPUT_SCALE: i32 = 64;
/// Largest absolute nonterminal score returned by this backend.
pub const MAX_SCORE: i32 = 16_000;

const _: () = assert!(MAX_SCORE < super::MATE_THRESHOLD);

const PIECES: [Piece; 6] = [
    Piece::Pawn,
    Piece::Knight,
    Piece::Bishop,
    Piece::Rook,
    Piece::Queen,
    Piece::King,
];
const COLORS: [Color; 2] = [Color::White, Color::Black];

/// Immutable, validated parameters for the fixed NNUE architecture.
///
/// Models can only be constructed by the bounded binary loaders. Mutable
/// accumulators borrow their model, so they cannot be reused with another set
/// of weights by accident. Loading never installs a model into an engine.
pub struct Network {
    hidden_bias: [i16; HIDDEN_SIZE],
    input_weights: Box<[i16]>,
    output_weights: [[i16; HIDDEN_SIZE]; 2],
    output_bias: i32,
}

impl fmt::Debug for Network {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Network")
            .field("input_features", &INPUT_FEATURES)
            .field("hidden_size", &HIDDEN_SIZE)
            .finish_non_exhaustive()
    }
}

impl Network {
    /// Fully recomputes the accumulators and returns a reference static score.
    #[must_use]
    pub fn evaluate(&self, board: &Board) -> i32 {
        self.accumulator(board).evaluate()
    }

    /// Builds independent mutable evaluation state bound to this model.
    #[must_use]
    pub fn accumulator(&self, board: &Board) -> Accumulator<'_> {
        let position = FeaturePosition::new(board);
        let sums = std::array::from_fn(|index| self.refresh(&position, COLORS[index]));
        Accumulator {
            network: self,
            position,
            sums,
        }
    }

    fn refresh(&self, position: &FeaturePosition, perspective: Color) -> [i32; HIDDEN_SIZE] {
        let mut sum = self.hidden_bias.map(i32::from);
        let bucket = position.buckets[color_index(perspective)];
        for (plane, &pieces) in position.pieces.iter().enumerate() {
            for square in BitBoard(pieces) {
                self.add_feature(&mut sum, feature(bucket, perspective, plane, square), 1);
            }
        }
        sum
    }

    #[inline]
    fn add_feature(&self, sum: &mut [i32; HIDDEN_SIZE], index: usize, sign: i32) {
        let row = &self.input_weights[index * HIDDEN_SIZE..(index + 1) * HIDDEN_SIZE];
        for (value, &weight) in sum.iter_mut().zip(row) {
            *value += sign * i32::from(weight);
        }
    }

    fn output(&self, sums: &[[i32; HIDDEN_SIZE]; 2], side_to_move: Color) -> i32 {
        let mut numerator = self.output_bias;
        for (weights, color) in self
            .output_weights
            .iter()
            .zip([side_to_move, !side_to_move])
        {
            for (&weight, &sum) in weights.iter().zip(&sums[color_index(color)]) {
                numerator += i32::from(weight) * sum.clamp(0, ACTIVATION_MAX);
            }
        }
        (numerator / (ACTIVATION_MAX * OUTPUT_SCALE)).clamp(-MAX_SCORE, MAX_SCORE)
    }
}

/// Incrementally maintained state for exactly one immutable network.
///
/// Updating from the saved piece placement, rather than from an assumed parent
/// move, also supports sibling positions, skipped evaluations and search retries.
/// A perspective is fully refreshed when its king bucket changes. Null moves
/// affect only output ordering; clocks, castling rights and en passant are not
/// features of this architecture.
#[derive(Clone)]
pub struct Accumulator<'network> {
    network: &'network Network,
    position: FeaturePosition,
    sums: [[i32; HIDDEN_SIZE]; 2],
}

impl Accumulator<'_> {
    /// Updates the exact unclipped sums to represent `board`.
    pub fn update(&mut self, board: &Board) {
        let next = FeaturePosition::new(board);
        for perspective in COLORS {
            let index = color_index(perspective);
            if next.buckets[index] != self.position.buckets[index] {
                self.sums[index] = self.network.refresh(&next, perspective);
                continue;
            }
            let bucket = next.buckets[index];
            // Removing before adding keeps intermediate sums within the same
            // 64-piece bound as a refresh, including unrelated sibling boards.
            for (plane, (&before, &after)) in
                self.position.pieces.iter().zip(&next.pieces).enumerate()
            {
                for square in BitBoard(before & !after) {
                    self.network.add_feature(
                        &mut self.sums[index],
                        feature(bucket, perspective, plane, square),
                        -1,
                    );
                }
            }
            for (plane, (&before, &after)) in
                self.position.pieces.iter().zip(&next.pieces).enumerate()
            {
                for square in BitBoard(after & !before) {
                    self.network.add_feature(
                        &mut self.sums[index],
                        feature(bucket, perspective, plane, square),
                        1,
                    );
                }
            }
        }
        self.position = next;
    }

    /// Returns the current side-to-move-relative static centipawn score.
    #[must_use]
    pub fn evaluate(&self) -> i32 {
        self.network.output(&self.sums, self.position.side_to_move)
    }

    /// Exposes the exact, unclipped sums for exporter and reference validation.
    #[must_use]
    pub fn values(&self, perspective: Color) -> &[i32; HIDDEN_SIZE] {
        &self.sums[color_index(perspective)]
    }
}

#[derive(Clone, Copy)]
struct FeaturePosition {
    // Absolute White P/N/B/R/Q/K, then Black P/N/B/R/Q/K.
    pieces: [u64; PIECE_PLANES],
    buckets: [usize; 2],
    side_to_move: Color,
}

impl FeaturePosition {
    fn new(board: &Board) -> Self {
        Self {
            pieces: std::array::from_fn(|plane| {
                board.colored_pieces(COLORS[plane / 6], PIECES[plane % 6]).0
            }),
            buckets: std::array::from_fn(|index| {
                king_bucket(board.king(COLORS[index]), COLORS[index])
            }),
            side_to_move: board.side_to_move(),
        }
    }
}

/// Returns the sorted, unique feature indices for one perspective.
///
/// The feature number is `(bucket * 12 + relative_piece_plane) * 64 + square`,
/// with a1 = 0, P/N/B/R/Q/K = 0..5, and enemy planes offset by six. Black's
/// perspective XORs square indices with 56 before selecting the bucket or row.
#[must_use]
pub fn active_features(board: &Board, perspective: Color) -> Vec<u16> {
    let position = FeaturePosition::new(board);
    let bucket = position.buckets[color_index(perspective)];
    let mut features = Vec::with_capacity(board.occupied().len() as usize);
    for (plane, &pieces) in position.pieces.iter().enumerate() {
        for square in BitBoard(pieces) {
            features.push(feature(bucket, perspective, plane, square) as u16);
        }
    }
    features.sort_unstable();
    features
}

fn color_index(color: Color) -> usize {
    match color {
        Color::White => 0,
        Color::Black => 1,
    }
}

fn oriented_square(square: Square, perspective: Color) -> usize {
    square as usize ^ if perspective == Color::Black { 56 } else { 0 }
}

fn king_bucket(king: Square, perspective: Color) -> usize {
    let square = oriented_square(king, perspective);
    (square / 8 / 2) * 4 + (square % 8 / 2)
}

fn feature(bucket: usize, perspective: Color, absolute_plane: usize, square: Square) -> usize {
    let owner = COLORS[absolute_plane / 6];
    let relative_plane = absolute_plane % 6 + 6 * usize::from(owner != perspective);
    (bucket * PIECE_PLANES + relative_plane) * 64 + oriented_square(square, perspective)
}
