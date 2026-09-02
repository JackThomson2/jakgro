use cozy_chess::Piece;

use super::{
    BISHOP_MOBILITY_ENTRIES, EvalFeatures, KNIGHT_MOBILITY_ENTRIES, QUEEN_MOBILITY_ENTRIES,
    ROOK_MOBILITY_ENTRIES, Score, ScorePair,
};

const PAWN: ScorePair = ScorePair::new(94, 149);
const KNIGHT: ScorePair = ScorePair::new(330, 290);
const BISHOP: ScorePair = ScorePair::new(347, 327);
const ROOK: ScorePair = ScorePair::new(503, 547);
const QUEEN: ScorePair = ScorePair::new(926, 932);
const ACTIVITY: ScorePair = ScorePair::new(3, -3);
const TEMPO: ScorePair = ScorePair::new(12, 0);
/// Weight per move for the two piece types without a mobility curve.
const PAWN_KING_MOBILITY: ScorePair = ScorePair::new(3, 7);
/// Mobility by move count, one entry per count a piece of that type can have.
///
/// These ship as exactly the linear term they replace: entry `n` is
/// `PAWN_KING_MOBILITY * n`, so adopting them moves no score. What they add is
/// the shape a fit may now give them — a knight's third square is not worth
/// what its eighth is, and one shared weight per move could never say so.
const KNIGHT_MOBILITY: [ScorePair; KNIGHT_MOBILITY_ENTRIES] = linear_mobility();
const BISHOP_MOBILITY: [ScorePair; BISHOP_MOBILITY_ENTRIES] = linear_mobility();
const ROOK_MOBILITY: [ScorePair; ROOK_MOBILITY_ENTRIES] = linear_mobility();
const QUEEN_MOBILITY: [ScorePair; QUEEN_MOBILITY_ENTRIES] = linear_mobility();

/// Builds a mobility curve that reproduces the shared linear weight.
const fn linear_mobility<const N: usize>() -> [ScorePair; N] {
    let mut curve = [ScorePair::new(0, 0); N];
    let mut count = 0;
    while count < N {
        curve[count] = ScorePair::new(
            PAWN_KING_MOBILITY.middle_game * count as Score,
            PAWN_KING_MOBILITY.end_game * count as Score,
        );
        count += 1;
    }
    curve
}
const PAWN_MOBILITY_ADJUSTMENT: ScorePair = ScorePair::new(-3, -2);
const KNIGHT_MOBILITY_ADJUSTMENT: ScorePair = ScorePair::new(1, 2);
const BISHOP_MOBILITY_ADJUSTMENT: ScorePair = ScorePair::new(2, 3);
const ROOK_MOBILITY_ADJUSTMENT: ScorePair = ScorePair::new(-1, 2);
const QUEEN_MOBILITY_ADJUSTMENT: ScorePair = ScorePair::new(-2, 0);
const KING_MOBILITY_ADJUSTMENT: ScorePair = ScorePair::new(-3, -2);
const BISHOP_PAIR: ScorePair = ScorePair::new(36, 47);
const DOUBLED_PAWN: ScorePair = ScorePair::new(-17, -21);
const ISOLATED_PAWN: ScorePair = ScorePair::new(-9, -10);
/// Passed pawn value by rank, from the owner's side of the board.
///
/// These six values are exactly `PASSED_PAWN * rank` for the single weight they
/// replace, which is why adopting them changes no score. The point is not the
/// numbers but the shape: the old term multiplied one weight by how far the
/// pawn had come, which forced the value of a passer to be a straight line
/// through the origin in its rank. A passer on the seventh is not seven times a
/// passer on the second, and a fit can now say so.
const PASSED_PAWN_BY_RANK: [ScorePair; 6] = [
    ScorePair::new(5, 16),
    ScorePair::new(10, 32),
    ScorePair::new(15, 48),
    ScorePair::new(20, 64),
    ScorePair::new(25, 80),
    ScorePair::new(30, 96),
];
/// Extra value for a passed pawn defended by a friendly pawn, by rank.
///
/// Zero until fitted, so this commit adds the feature without moving a score.
const PROTECTED_PASSED_PAWN_BY_RANK: [ScorePair; 6] = [ScorePair::new(0, 0); 6];
const KING_SHELTER: ScorePair = ScorePair::new(23, -12);
const OPEN_KING_FILE: ScorePair = ScorePair::new(-18, -3);
/// Rook placement, zero until fitted so adding the terms moves no score.
///
/// A hand-set file bonus was screened once and reversed sign on a holdout,
/// which is the case for letting the data set it rather than for leaving the
/// term out.
const ROOK_OPEN_FILE: ScorePair = ScorePair::new(0, 0);
const ROOK_SEMI_OPEN_FILE: ScorePair = ScorePair::new(0, 0);
const ROOK_ON_SEVENTH: ScorePair = ScorePair::new(0, 0);
/// Minor pieces on outposts, zero until fitted.
const KNIGHT_OUTPOST: ScorePair = ScorePair::new(0, 0);
const BISHOP_OUTPOST: ScorePair = ScorePair::new(0, 0);
/// Pawn structure beyond doubled and isolated, zero until fitted.
const BACKWARD_PAWN: ScorePair = ScorePair::new(0, 0);
const CONNECTED_PAWN_BY_RANK: [ScorePair; 6] = [ScorePair::new(0, 0); 6];
/// Passer refinements, zero until fitted: a blockade by rank, and the
/// distance of each king to the square ahead of the passer.
const BLOCKED_PASSER_BY_RANK: [ScorePair; 6] = [ScorePair::new(0, 0); 6];
const PASSER_OWN_KING_DISTANCE: [ScorePair; 8] = [ScorePair::new(0, 0); 8];
const PASSER_ENEMY_KING_DISTANCE: [ScorePair; 8] = [ScorePair::new(0, 0); 8];
const KING_PRESSURE: ScorePair = ScorePair::new(9, 2);
const PAWN_STORM: ScorePair = ScorePair::new(7, 1);
const THREAT: ScorePair = ScorePair::new(11, 7);
const SPACE: ScorePair = ScorePair::new(2, 0);
const PASSER_URGENCY: ScorePair = ScorePair::new(3, 6);
const COORDINATION: ScorePair = ScorePair::new(16, 2);
const SUPPORTED_THREAT: ScorePair = ScorePair::new(18, 5);
const OPEN_LINE: ScorePair = ScorePair::new(14, 1);
const PAWN_BREAK: ScorePair = ScorePair::new(12, 1);

pub(super) fn score(features: EvalFeatures) -> ScorePair {
    PAWN * features.pawns
        + KNIGHT * features.knights
        + BISHOP * features.bishops
        + ROOK * features.rooks
        + QUEEN * features.queens
        + features.placement
        + ACTIVITY * features.activity
        + TEMPO * features.tempo
        + PAWN_KING_MOBILITY * (features.pawn_mobility + features.king_mobility)
        + features.mobility_curves
        + BISHOP_PAIR * features.bishop_pair
        + DOUBLED_PAWN * features.doubled_pawns
        + ISOLATED_PAWN * features.isolated_pawns
        + indexed(&PASSED_PAWN_BY_RANK, features.passed_by_rank)
        + indexed(
            &PROTECTED_PASSED_PAWN_BY_RANK,
            features.protected_passer_by_rank,
        )
        + KING_SHELTER * features.king_shelter
        + OPEN_KING_FILE * features.open_king_files
        + ROOK_OPEN_FILE * features.rook_open_files
        + ROOK_SEMI_OPEN_FILE * features.rook_semi_open_files
        + ROOK_ON_SEVENTH * features.rooks_on_seventh
        + KNIGHT_OUTPOST * features.knight_outposts
        + BISHOP_OUTPOST * features.bishop_outposts
        + BACKWARD_PAWN * features.backward_pawns
        + indexed(&CONNECTED_PAWN_BY_RANK, features.connected_by_rank)
        + indexed(&BLOCKED_PASSER_BY_RANK, features.blocked_passer_by_rank)
        + indexed(&PASSER_OWN_KING_DISTANCE, features.passer_own_king_distance)
        + indexed(
            &PASSER_ENEMY_KING_DISTANCE,
            features.passer_enemy_king_distance,
        )
}

/// The four curves laid end to end, which is how the piece loop reads them.
static MOBILITY_CURVES: [ScorePair; MOBILITY_CURVE_ENTRIES] = concatenated_curves();

/// Entries across the four mobility curves.
const MOBILITY_CURVE_ENTRIES: usize = KNIGHT_MOBILITY_ENTRIES
    + BISHOP_MOBILITY_ENTRIES
    + ROOK_MOBILITY_ENTRIES
    + QUEEN_MOBILITY_ENTRIES;

const fn concatenated_curves() -> [ScorePair; MOBILITY_CURVE_ENTRIES] {
    let mut flat = [ScorePair::new(0, 0); MOBILITY_CURVE_ENTRIES];
    let curves: [&[ScorePair]; 4] = [
        &KNIGHT_MOBILITY,
        &BISHOP_MOBILITY,
        &ROOK_MOBILITY,
        &QUEEN_MOBILITY,
    ];
    let mut next = 0;
    let mut curve = 0;
    while curve < curves.len() {
        let mut index = 0;
        while index < curves[curve].len() {
            flat[next] = curves[curve][index];
            next += 1;
            index += 1;
        }
        curve += 1;
    }
    flat
}

/// Returns where a piece's mobility curve starts, if it has one.
///
/// Pawns and kings have no curve and score through [`score`]'s shared weight:
/// a pawn's "mobility" is its pushes and captures, which the structure terms
/// describe better, and a king's is its exposure as much as its freedom. The
/// piece loop asks once per piece type, so the half of the pieces without a
/// curve cost nothing per square.
pub(super) const fn mobility_curve_offset(piece: Piece) -> Option<usize> {
    match piece {
        Piece::Knight => Some(0),
        Piece::Bishop => Some(KNIGHT_MOBILITY_ENTRIES),
        Piece::Rook => Some(KNIGHT_MOBILITY_ENTRIES + BISHOP_MOBILITY_ENTRIES),
        Piece::Queen => {
            Some(KNIGHT_MOBILITY_ENTRIES + BISHOP_MOBILITY_ENTRIES + ROOK_MOBILITY_ENTRIES)
        }
        Piece::Pawn | Piece::King => None,
    }
}

/// Returns the weight at a curve offset plus a move count.
pub(super) fn mobility_curve_at(index: usize) -> ScorePair {
    MOBILITY_CURVES[index]
}

/// Returns the mobility weight for a piece with the given number of moves.
#[cfg(test)]
pub(super) fn mobility_curve(piece: Piece, count: usize) -> ScorePair {
    match mobility_curve_offset(piece) {
        Some(offset) => mobility_curve_at(offset + count),
        None => ScorePair::new(0, 0),
    }
}

/// Returns the dot product of an indexed weight block with its counts.
fn indexed<const N: usize>(weights: &[ScorePair; N], features: [Score; N]) -> ScorePair {
    let mut total = ScorePair::new(0, 0);
    for index in 0..N {
        total = total + weights[index] * features[index];
    }
    total
}

pub(super) fn profile_mobility_adjustment(features: EvalFeatures) -> ScorePair {
    PAWN_MOBILITY_ADJUSTMENT * features.pawn_mobility
        + KNIGHT_MOBILITY_ADJUSTMENT * features.knight_mobility
        + BISHOP_MOBILITY_ADJUSTMENT * features.bishop_mobility
        + ROOK_MOBILITY_ADJUSTMENT * features.rook_mobility
        + QUEEN_MOBILITY_ADJUSTMENT * features.queen_mobility
        + KING_MOBILITY_ADJUSTMENT * features.king_mobility
}

pub(super) fn attacking_style(features: EvalFeatures) -> ScorePair {
    KING_PRESSURE * features.king_pressure
        + PAWN_STORM * features.pawn_storm
        + THREAT * features.threats
        + SPACE * features.space
        + PASSER_URGENCY * features.passed_pawns
        + COORDINATION * features.coordination
        + SUPPORTED_THREAT * features.supported_threats
        + OPEN_LINE * features.open_lines
        + PAWN_BREAK * features.pawn_breaks
}

/// Returns the scalar weights in the order the tuning feature vector uses.
///
/// The order is the one [`score`] combines them in, so a fitted value maps back
/// onto exactly one constant above without an intervening table.
#[cfg(feature = "tuning")]
pub(super) const fn tuning_weights() -> [ScorePair; super::tuning::SCALAR_FEATURES] {
    [
        PAWN,
        KNIGHT,
        BISHOP,
        ROOK,
        QUEEN,
        ACTIVITY,
        TEMPO,
        PAWN_KING_MOBILITY,
        BISHOP_PAIR,
        DOUBLED_PAWN,
        ISOLATED_PAWN,
        PASSED_PAWN_BY_RANK[0],
        PASSED_PAWN_BY_RANK[1],
        PASSED_PAWN_BY_RANK[2],
        PASSED_PAWN_BY_RANK[3],
        PASSED_PAWN_BY_RANK[4],
        PASSED_PAWN_BY_RANK[5],
        PROTECTED_PASSED_PAWN_BY_RANK[0],
        PROTECTED_PASSED_PAWN_BY_RANK[1],
        PROTECTED_PASSED_PAWN_BY_RANK[2],
        PROTECTED_PASSED_PAWN_BY_RANK[3],
        PROTECTED_PASSED_PAWN_BY_RANK[4],
        PROTECTED_PASSED_PAWN_BY_RANK[5],
        KING_SHELTER,
        OPEN_KING_FILE,
    ]
}

/// Returns the weights of the blocks that follow the placement tables.
///
/// Groups added after the tables live here, in the order [`score`] reads them,
/// so the vector's leading indices keep their meaning across additions.
#[cfg(feature = "tuning")]
pub(super) fn trailing_tuning_weights() -> [ScorePair; super::tuning::TRAILING_FEATURES] {
    let mut weights = [ScorePair::new(0, 0); super::tuning::TRAILING_FEATURES];
    let mut next = 0;
    for block in [
        &MOBILITY_CURVES[..],
        &trailing_scalars()[..],
        &CONNECTED_PAWN_BY_RANK[..],
        &BLOCKED_PASSER_BY_RANK[..],
        &PASSER_OWN_KING_DISTANCE[..],
        &PASSER_ENEMY_KING_DISTANCE[..],
    ] {
        weights[next..next + block.len()].copy_from_slice(block);
        next += block.len();
    }
    debug_assert_eq!(next, weights.len());
    weights
}

/// The scalar weights after the mobility curves, in the order [`score`] reads
/// them and [`super::tuning::BLOCKS`] declares them.
#[cfg(feature = "tuning")]
pub(super) const fn trailing_scalars() -> [ScorePair; super::tuning::TRAILING_SCALARS] {
    [
        ROOK_OPEN_FILE,
        ROOK_SEMI_OPEN_FILE,
        ROOK_ON_SEVENTH,
        KNIGHT_OUTPOST,
        BISHOP_OUTPOST,
        BACKWARD_PAWN,
    ]
}
