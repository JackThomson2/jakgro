//! Linear feature vector for offline weight fitting.
//!
//! The objective evaluation is a dot product. [`super::weights::score`] combines
//! fourteen scalar features with one weight pair each, and adds a placement term
//! that is itself a sum of piece-square entries. Written out, the whole thing is
//!
//! ```text
//! score_mg = Σ w_mg[i] · x[i]      score_eg = Σ w_eg[i] · x[i]
//! ```
//!
//! over one vector `x` that this module produces, with the placement term
//! expanded from a precomputed pair back into the per-piece, per-square counts it
//! was summed from. A fitter can then treat the evaluation as ordinary linear
//! regression, and the engine and the fitter share one extraction rather than
//! two implementations that must be kept in agreement.
//!
//! What is deliberately *not* here: the profile mobility adjustment, which is
//! scaled by a per-profile intensity and so is not a fixed part of the objective
//! score, and the attacking-style weights, which are the personality. Both stay
//! exactly as written. Fitting them to game results would tune the engine's
//! character out of it, because game results do not reward interesting chess.
//!
//! This module is behind the `tuning` feature and is not built into the engine.

use cozy_chess::{Board, Color, Piece, Square};

use super::{Score, features, placement, weights};

/// Scalar features, in the order [`super::weights::score`] combines them.
pub const SCALAR_FEATURES: usize = 14;
/// Piece-square entries: six pieces over sixty-four squares.
pub const PLACEMENT_FEATURES: usize = 6 * 64;
/// Length of the feature vector.
pub const FEATURE_COUNT: usize = SCALAR_FEATURES + PLACEMENT_FEATURES;
/// Index of the first piece-square feature.
pub const PLACEMENT_OFFSET: usize = SCALAR_FEATURES;
/// Largest phase value, at which the middlegame weight applies alone.
pub const MAX_PHASE: i32 = 24;

/// One position's features, as the non-zero entries of a sparse vector.
///
/// Most of the vector is zero: at most thirty-two of the three hundred and
/// eighty-four placement features can be set, and several scalars are usually
/// zero too. A fitter walks millions of these, so the sparse form is what keeps
/// a pass over the corpus bounded by the pieces on the board.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TuningPosition {
    /// Feature index and its White-positive count.
    pub entries: Vec<(u16, i16)>,
    /// Game phase, from zero in a bare ending to [`MAX_PHASE`].
    pub phase: i32,
}

impl TuningPosition {
    /// Returns the blended score this vector produces under the given weights.
    ///
    /// This is the model the fitter optimizes, written once so the fit and the
    /// verification cannot drift apart.
    #[must_use]
    pub fn score(&self, weights: &[(Score, Score)]) -> Score {
        let mut middle_game = 0;
        let mut end_game = 0;
        for &(index, count) in &self.entries {
            let (weight_mg, weight_eg) = weights[index as usize];
            middle_game += weight_mg * Score::from(count);
            end_game += weight_eg * Score::from(count);
        }
        (middle_game * self.phase + end_game * (MAX_PHASE - self.phase)) / MAX_PHASE
    }
}

/// Extracts the linear feature vector for a position, from White's perspective.
#[must_use]
pub fn tuning_features(board: &Board) -> TuningPosition {
    let extracted = features::extract_with_style(board, false);
    let scalars = [
        extracted.pawns,
        extracted.knights,
        extracted.bishops,
        extracted.rooks,
        extracted.queens,
        extracted.activity,
        extracted.tempo,
        extracted.mobility,
        extracted.bishop_pair,
        extracted.doubled_pawns,
        extracted.isolated_pawns,
        extracted.passed_pawns,
        extracted.king_shelter,
        extracted.open_king_files,
    ];

    let mut entries = Vec::with_capacity(SCALAR_FEATURES + 32);
    for (index, value) in scalars.into_iter().enumerate() {
        if value != 0 {
            entries.push((index as u16, value as i16));
        }
    }

    // Placement is accumulated per piece and square rather than read back from
    // the fused pair, which is what turns one blended number into the hundreds
    // of parameters that produced it.
    let mut placement_counts = [0_i16; PLACEMENT_FEATURES];
    for color in [Color::White, Color::Black] {
        let sign = if color == Color::White { 1 } else { -1 };
        for piece in Piece::ALL {
            for square in board.colored_pieces(color, piece) {
                placement_counts[placement_feature(piece, square, color)] += sign;
            }
        }
    }
    for (offset, count) in placement_counts.into_iter().enumerate() {
        if count != 0 {
            entries.push(((PLACEMENT_OFFSET + offset) as u16, count));
        }
    }

    TuningPosition {
        entries,
        phase: features::phase(board),
    }
}

/// Returns the weight vector the engine currently ships, in feature order.
#[must_use]
pub fn current_weights() -> Vec<(Score, Score)> {
    let mut weights: Vec<(Score, Score)> = weights::tuning_weights()
        .into_iter()
        .map(|pair| (pair.middle_game(), pair.end_game()))
        .collect();
    weights.extend(Piece::ALL.into_iter().flat_map(|piece| {
        (0..64).map(move |index| {
            let entry = placement::table_entry(piece, index);
            (entry.middle_game(), entry.end_game())
        })
    }));
    debug_assert_eq!(weights.len(), FEATURE_COUNT);
    weights
}

/// Maps a piece on a square to its placement feature index.
///
/// The index is the table row the engine reads, so a fitted weight can be
/// written straight back into the published table without a second mapping.
fn placement_feature(piece: Piece, square: Square, color: Color) -> usize {
    piece as usize * 64 + placement::table_index_for_tuning(square, color)
}

/// Returns a weight vector as the two source tables the engine reads.
///
/// Piece-square tables and material values are jointly under-determined: adding
/// a constant to every entry of one table and subtracting it from that piece's
/// material weight leaves every score unchanged. A fit therefore lands anywhere
/// along that ridge. Re-centring each table on zero and folding the mean it
/// carried into the material weight picks the one point on the ridge where the
/// tables say only where a piece belongs and the material weights say only what
/// it is worth, which is what both are documented to mean.
///
/// The king has no material weight, so the constant its table carries is not
/// merely arbitrary, it is unobservable: both sides always have exactly one, so
/// it cancels. It is centred and discarded.
#[must_use]
pub fn normalized(weights: &[(Score, Score)]) -> Vec<(Score, Score)> {
    let mut normalized = weights.to_vec();
    for piece in Piece::ALL {
        let start = PLACEMENT_OFFSET + piece as usize * 64;
        let squares = &mut normalized[start..start + 64];
        let occupiable: Vec<usize> = (0..64)
            .filter(|&index| piece != Piece::Pawn || (8..56).contains(&index))
            .collect();
        let count = occupiable.len() as Score;
        let mean_mg = occupiable.iter().map(|&i| squares[i].0).sum::<Score>() / count;
        let mean_eg = occupiable.iter().map(|&i| squares[i].1).sum::<Score>() / count;
        for &index in &occupiable {
            squares[index].0 -= mean_mg;
            squares[index].1 -= mean_eg;
        }
        // A pawn can never stand on the first or last rank, so those rows are
        // structurally zero rather than merely unvisited.
        if piece == Piece::Pawn {
            for index in (0..8).chain(56..64) {
                squares[index] = (0, 0);
            }
        }
        if let Some(material) = material_feature(piece) {
            normalized[material].0 += mean_mg;
            normalized[material].1 += mean_eg;
        }
    }
    normalized
}

/// Returns the scalar feature holding a piece's material weight, if it has one.
const fn material_feature(piece: Piece) -> Option<usize> {
    match piece {
        Piece::Pawn => Some(0),
        Piece::Knight => Some(1),
        Piece::Bishop => Some(2),
        Piece::Rook => Some(3),
        Piece::Queen => Some(4),
        Piece::King => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{FEATURE_COUNT, current_weights, normalized, tuning_features};
    use crate::engine::Position;
    use crate::engine::evaluation::{EvaluationConfig, MIN_AGGRESSION, weights};

    const POSITIONS: [&str; 6] = [
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
        "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        "r1bq1rk1/ppp2ppp/2n2n2/2b1p3/2B1P3/2NP1N2/PPP2PPP/R1BQ1RK1 w - - 0 8",
        "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
        "6k1/5ppp/8/7Q/2B5/8/5PPP/6K1 w - - 0 1",
        "4k3/8/8/8/8/8/4q3/4R1K1 b - - 0 1",
    ];

    /// The vector and the engine must agree exactly, or a fit optimizes a model
    /// the engine does not use.
    #[test]
    fn the_feature_vector_reproduces_the_objective_score() {
        let weights_now = current_weights();
        assert_eq!(weights_now.len(), FEATURE_COUNT);

        for fen in POSITIONS {
            let position = Position::from_fen(fen).unwrap();
            let extracted = super::features::extract_with_style(position.board(), false);
            let expected = weights::score(extracted);
            let vector = tuning_features(position.board());

            let mut middle_game = 0;
            let mut end_game = 0;
            for &(index, count) in &vector.entries {
                let (weight_mg, weight_eg) = weights_now[index as usize];
                middle_game += weight_mg * i32::from(count);
                end_game += weight_eg * i32::from(count);
            }

            assert_eq!(
                (middle_game, end_game),
                (expected.middle_game(), expected.end_game()),
                "feature vector disagreed on {fen}",
            );
        }
    }

    /// Re-centring moves weight between the tables and the material values
    /// without moving any score.
    #[test]
    fn normalization_preserves_every_score() {
        let before = current_weights();
        let after = normalized(&before);
        assert_ne!(
            before, after,
            "the published tables are not already centred"
        );

        for fen in POSITIONS {
            let position = Position::from_fen(fen).unwrap();
            let vector = tuning_features(position.board());
            assert_eq!(
                vector.score(&before),
                vector.score(&after),
                "normalization moved the score of {fen}",
            );
        }
    }

    /// The blended model must match what search actually computes at the profile
    /// the fit targets.
    #[test]
    fn the_blended_model_matches_the_objective_evaluation() {
        let weights_now = current_weights();
        let objective = EvaluationConfig::new(MIN_AGGRESSION);

        for fen in POSITIONS {
            let position = Position::from_fen(fen).unwrap();
            let board = position.board();
            let modelled = tuning_features(board).score(&weights_now);
            let engine = super::super::evaluate_with_trace_and_config(board, objective).blended;

            assert_eq!(modelled, engine, "model disagreed with evaluation on {fen}");
        }
    }
}
