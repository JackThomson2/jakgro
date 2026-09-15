//! Experimental, standalone quantized neural evaluation.
//!
//! This module does not select an engine backend or supply a default network.
//! Features are colored piece-square pairs, including both kings, conditioned
//! on one of eight 2x2 buckets of the perspective king; one of eight output
//! layers is selected by the number of pieces on the board. Black's perspective
//! mirrors ranks; a perspective whose king stands on files e-h mirrors files as
//! well, so the king always lies on files a-d and every position shares
//! weights with its horizontal reflection. Both perspectives share the feature
//! transformer.
//! The output concatenates the side-to-move accumulator before its opponent's.
//!
//! Accumulators contain unclipped integer sums. Inference clips each activation
//! to 0..=255, computes an integer dot product, and divides by 255 * 64 with
//! truncation toward zero. The result is side-to-move-relative centipawns,
//! bounded below the engine's mate region. It is not a terminal-position score.

mod format;
mod kernels;
#[cfg(test)]
mod tests;

use std::fmt;

use cozy_chess::{BitBoard, Board, Color, Piece, Square};

pub use format::{FILE_BYTES, FORMAT_VERSION, HEADER_BYTES, LoadError, MAGIC};

/// Number of 2x2 king buckets on the oriented, file-mirrored half board.
pub const KING_BUCKETS: usize = 8;
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
/// Output layers selected by piece count: `(pieces - 1) / 4` for 2..=32 pieces.
pub const OUTPUT_BUCKETS: usize = 8;
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
    /// Per output bucket: side-to-move row, then opponent row.
    output_weights: [[[i16; HIDDEN_SIZE]; 2]; OUTPUT_BUCKETS],
    output_bias: [i32; OUTPUT_BUCKETS],
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
        let view = position.views[color_index(perspective)];
        for (plane, &pieces) in position.pieces.iter().enumerate() {
            for square in BitBoard(pieces) {
                self.add_feature(&mut sum, feature(view, perspective, plane, square));
            }
        }
        sum
    }

    #[inline]
    fn row(&self, index: usize) -> &[i16] {
        &self.input_weights[index * HIDDEN_SIZE..(index + 1) * HIDDEN_SIZE]
    }

    #[inline]
    fn add_feature(&self, sum: &mut [i32; HIDDEN_SIZE], index: usize) {
        (kernels::kernels().add)(sum, self.row(index));
    }

    #[inline]
    fn remove_feature(&self, sum: &mut [i32; HIDDEN_SIZE], index: usize) {
        (kernels::kernels().sub)(sum, self.row(index));
    }

    fn output(&self, sums: &[[i32; HIDDEN_SIZE]; 2], side_to_move: Color, bucket: usize) -> i32 {
        let dot = kernels::kernels().dot;
        let mut numerator = self.output_bias[bucket];
        for (weights, color) in self.output_weights[bucket]
            .iter()
            .zip([side_to_move, !side_to_move])
        {
            numerator += dot(weights, &sums[color_index(color)]);
        }
        (numerator / (ACTIVATION_MAX * OUTPUT_SCALE)).clamp(-MAX_SCORE, MAX_SCORE)
    }
}

/// Incrementally maintained state for exactly one immutable network.
///
/// Updating from the saved piece placement, rather than from an assumed parent
/// move, also supports sibling positions, skipped evaluations and search retries.
/// A perspective is fully refreshed when its king bucket or mirroring changes. Null moves
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
            if next.views[index] != self.position.views[index] {
                self.sums[index] = self.network.refresh(&next, perspective);
                continue;
            }
            let view = next.views[index];
            // Removing before adding keeps intermediate sums within the same
            // 64-piece bound as a refresh, including unrelated sibling boards.
            for (plane, (&before, &after)) in
                self.position.pieces.iter().zip(&next.pieces).enumerate()
            {
                for square in BitBoard(before & !after) {
                    self.network.remove_feature(
                        &mut self.sums[index],
                        feature(view, perspective, plane, square),
                    );
                }
            }
            for (plane, (&before, &after)) in
                self.position.pieces.iter().zip(&next.pieces).enumerate()
            {
                for square in BitBoard(after & !before) {
                    self.network.add_feature(
                        &mut self.sums[index],
                        feature(view, perspective, plane, square),
                    );
                }
            }
        }
        self.position = next;
    }

    /// Returns the current side-to-move-relative static centipawn score.
    #[must_use]
    pub fn evaluate(&self) -> i32 {
        self.network.output(
            &self.sums,
            self.position.side_to_move,
            self.position.output_bucket,
        )
    }

    /// Exposes the exact, unclipped sums for exporter and reference validation.
    #[must_use]
    pub fn values(&self, perspective: Color) -> &[i32; HIDDEN_SIZE] {
        &self.sums[color_index(perspective)]
    }
}

/// How one perspective sees the board: its king bucket and whether files are
/// mirrored so that the king lies on files a-d.
#[derive(Clone, Copy, PartialEq, Eq)]
struct View {
    bucket: usize,
    mirror: bool,
}

#[derive(Clone, Copy)]
struct FeaturePosition {
    // Absolute White P/N/B/R/Q/K, then Black P/N/B/R/Q/K.
    pieces: [u64; PIECE_PLANES],
    views: [View; 2],
    side_to_move: Color,
    output_bucket: usize,
}

impl FeaturePosition {
    fn new(board: &Board) -> Self {
        Self {
            pieces: std::array::from_fn(|plane| {
                board.colored_pieces(COLORS[plane / 6], PIECES[plane % 6]).0
            }),
            views: std::array::from_fn(|index| view(board.king(COLORS[index]), COLORS[index])),
            side_to_move: board.side_to_move(),
            output_bucket: output_bucket(board),
        }
    }
}

/// Returns the sorted, unique feature indices for one perspective.
///
/// The feature number is `(bucket * 12 + relative_piece_plane) * 64 + square`,
/// with a1 = 0, P/N/B/R/Q/K = 0..5, and enemy planes offset by six. Black's
/// perspective XORs square indices with 56; a perspective whose king is on
/// files e-h XORs them with 7 as well, before selecting the bucket or row.
#[must_use]
pub fn active_features(board: &Board, perspective: Color) -> Vec<u16> {
    let position = FeaturePosition::new(board);
    let view = position.views[color_index(perspective)];
    let mut features = Vec::with_capacity(board.occupied().len() as usize);
    for (plane, &pieces) in position.pieces.iter().enumerate() {
        for square in BitBoard(pieces) {
            features.push(feature(view, perspective, plane, square) as u16);
        }
    }
    features.sort_unstable();
    features
}

/// Selects the output layer from the piece count; both kings are always present.
#[must_use]
pub fn output_bucket(board: &Board) -> usize {
    (board.occupied().len() as usize - 1) / 4
}

fn color_index(color: Color) -> usize {
    match color {
        Color::White => 0,
        Color::Black => 1,
    }
}

fn oriented_square(square: Square, perspective: Color, mirror: bool) -> usize {
    let flip = if perspective == Color::Black { 56 } else { 0 } | if mirror { 7 } else { 0 };
    square as usize ^ flip
}

fn view(king: Square, perspective: Color) -> View {
    let mirror = king.file() as usize >= 4;
    let square = oriented_square(king, perspective, mirror);
    View {
        bucket: (square / 8 / 2) * 2 + (square % 8 / 2),
        mirror,
    }
}

fn feature(view: View, perspective: Color, absolute_plane: usize, square: Square) -> usize {
    let owner = COLORS[absolute_plane / 6];
    let relative_plane = absolute_plane % 6 + 6 * usize::from(owner != perspective);
    (view.bucket * PIECE_PLANES + relative_plane) * 64
        + oriented_square(square, perspective, view.mirror)
}
