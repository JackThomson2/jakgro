use cozy_chess::Piece;

use super::features::StructureCounts;
use super::{
    BISHOP_MOBILITY_ENTRIES, EvalFeatures, KING_DANGER_BUCKETS, KNIGHT_MOBILITY_ENTRIES,
    QUEEN_MOBILITY_ENTRIES, ROOK_MOBILITY_ENTRIES, Score, ScorePair,
};

const PAWN: ScorePair = ScorePair::new(94, 149);
const KNIGHT: ScorePair = ScorePair::new(330, 290);
const BISHOP: ScorePair = ScorePair::new(347, 327);
const ROOK: ScorePair = ScorePair::new(503, 547);
const QUEEN: ScorePair = ScorePair::new(926, 932);
const ACTIVITY: ScorePair = ScorePair::new(2, -5);
const TEMPO: ScorePair = ScorePair::new(21, 2);
/// Weight per move for the two piece types without a mobility curve.
const PAWN_KING_MOBILITY: ScorePair = ScorePair::new(-1, -3);
/// Mobility by move count, one entry per count a piece of that type can have.
///
/// Fitted curves rather than one weight per move: a knight's third square is
/// not worth what its eighth is, and a trapped piece costs more than a line
/// through the origin can express.
const KNIGHT_MOBILITY: [ScorePair; 9] = [
    ScorePair::new(-2, 0),
    ScorePair::new(4, 9),
    ScorePair::new(11, 22),
    ScorePair::new(17, 28),
    ScorePair::new(16, 37),
    ScorePair::new(19, 40),
    ScorePair::new(18, 45),
    ScorePair::new(21, 49),
    ScorePair::new(19, 49),
];
const BISHOP_MOBILITY: [ScorePair; 14] = [
    ScorePair::new(-11, -3),
    ScorePair::new(-1, 8),
    ScorePair::new(11, 18),
    ScorePair::new(10, 27),
    ScorePair::new(14, 35),
    ScorePair::new(24, 46),
    ScorePair::new(25, 51),
    ScorePair::new(31, 57),
    ScorePair::new(24, 62),
    ScorePair::new(25, 70),
    ScorePair::new(29, 73),
    ScorePair::new(32, 75),
    ScorePair::new(40, 92),
    ScorePair::new(43, 95),
];
const ROOK_MOBILITY: [ScorePair; 15] = [
    ScorePair::new(-14, -3),
    ScorePair::new(0, 10),
    ScorePair::new(6, 21),
    ScorePair::new(16, 31),
    ScorePair::new(21, 36),
    ScorePair::new(21, 43),
    ScorePair::new(19, 57),
    ScorePair::new(24, 61),
    ScorePair::new(25, 69),
    ScorePair::new(29, 74),
    ScorePair::new(31, 79),
    ScorePair::new(31, 82),
    ScorePair::new(36, 87),
    ScorePair::new(41, 94),
    ScorePair::new(40, 81),
];
const QUEEN_MOBILITY: [ScorePair; 28] = [
    ScorePair::new(0, 0),
    ScorePair::new(5, 7),
    ScorePair::new(8, 16),
    ScorePair::new(15, 23),
    ScorePair::new(16, 32),
    ScorePair::new(21, 40),
    ScorePair::new(25, 48),
    ScorePair::new(29, 58),
    ScorePair::new(32, 65),
    ScorePair::new(33, 73),
    ScorePair::new(35, 81),
    ScorePair::new(36, 90),
    ScorePair::new(42, 96),
    ScorePair::new(47, 107),
    ScorePair::new(52, 114),
    ScorePair::new(50, 120),
    ScorePair::new(56, 129),
    ScorePair::new(55, 136),
    ScorePair::new(58, 142),
    ScorePair::new(63, 149),
    ScorePair::new(66, 156),
    ScorePair::new(69, 163),
    ScorePair::new(73, 171),
    ScorePair::new(77, 178),
    ScorePair::new(80, 187),
    ScorePair::new(84, 196),
    ScorePair::new(88, 204),
    ScorePair::new(91, 212),
];
const PAWN_MOBILITY_ADJUSTMENT: ScorePair = ScorePair::new(-3, -2);
const KNIGHT_MOBILITY_ADJUSTMENT: ScorePair = ScorePair::new(1, 2);
const BISHOP_MOBILITY_ADJUSTMENT: ScorePair = ScorePair::new(2, 3);
const ROOK_MOBILITY_ADJUSTMENT: ScorePair = ScorePair::new(-1, 2);
const QUEEN_MOBILITY_ADJUSTMENT: ScorePair = ScorePair::new(-2, 0);
const KING_MOBILITY_ADJUSTMENT: ScorePair = ScorePair::new(-3, -2);
const BISHOP_PAIR: ScorePair = ScorePair::new(37, 52);
const DOUBLED_PAWN: ScorePair = ScorePair::new(-11, -19);
const ISOLATED_PAWN: ScorePair = ScorePair::new(-5, -7);
/// Passed pawn value by rank, from the owner's side of the board.
///
/// Six fitted values rather than one weight times the rank: a passer on the
/// seventh is not seven times a passer on the second.
const PASSED_PAWN_BY_RANK: [ScorePair; 6] = [
    ScorePair::new(-1, 10),
    ScorePair::new(2, 26),
    ScorePair::new(10, 60),
    ScorePair::new(24, 87),
    ScorePair::new(37, 97),
    ScorePair::new(32, 89),
];
/// Extra value for a passed pawn defended by a friendly pawn, by rank.
const PROTECTED_PASSED_PAWN_BY_RANK: [ScorePair; 6] = [
    ScorePair::new(0, 0),
    ScorePair::new(0, -5),
    ScorePair::new(0, 0),
    ScorePair::new(7, 2),
    ScorePair::new(6, 4),
    ScorePair::new(0, -2),
];
const KING_SHELTER: ScorePair = ScorePair::new(25, -14);
const OPEN_KING_FILE: ScorePair = ScorePair::new(-16, -4);
/// Rook placement. A hand-set file bonus was screened once and reversed sign
/// on a holdout; these are the data's values.
const ROOK_OPEN_FILE: ScorePair = ScorePair::new(27, -9);
const ROOK_SEMI_OPEN_FILE: ScorePair = ScorePair::new(3, 19);
const ROOK_ON_SEVENTH: ScorePair = ScorePair::new(0, 8);
/// Minor pieces on outposts.
const KNIGHT_OUTPOST: ScorePair = ScorePair::new(18, 8);
const BISHOP_OUTPOST: ScorePair = ScorePair::new(4, 9);
/// Pawn structure beyond doubled and isolated.
const BACKWARD_PAWN: ScorePair = ScorePair::new(-1, -5);
/// Threats in the objective evaluation. The style's own threat terms are
/// untouched.
const THREAT_MINOR_BY_PAWN: ScorePair = ScorePair::new(36, 16);
const THREAT_HANGING: ScorePair = ScorePair::new(23, 20);
const THREAT_BY_LOWER_VALUE: ScorePair = ScorePair::new(31, 11);
const CONNECTED_PAWN_BY_RANK: [ScorePair; 6] = [
    ScorePair::new(3, -2),
    ScorePair::new(12, 8),
    ScorePair::new(14, 12),
    ScorePair::new(20, 17),
    ScorePair::new(8, 10),
    ScorePair::new(0, -1),
];
/// Passer refinements: a blockade by rank, and the distance of each king to
/// the square ahead of the passer.
const BLOCKED_PASSER_BY_RANK: [ScorePair; 6] = [
    ScorePair::new(-3, -7),
    ScorePair::new(-3, -3),
    ScorePair::new(-8, -5),
    ScorePair::new(-3, -12),
    ScorePair::new(-2, -15),
    ScorePair::new(-6, -21),
];
const PASSER_OWN_KING_DISTANCE: [ScorePair; 8] = [
    ScorePair::new(0, 5),
    ScorePair::new(2, 25),
    ScorePair::new(-2, 7),
    ScorePair::new(-2, -2),
    ScorePair::new(-8, -10),
    ScorePair::new(-5, -14),
    ScorePair::new(3, -9),
    ScorePair::new(-4, -8),
];
const PASSER_ENEMY_KING_DISTANCE: [ScorePair; 8] = [
    ScorePair::new(-11, -30),
    ScorePair::new(-4, -34),
    ScorePair::new(-3, -16),
    ScorePair::new(2, 5),
    ScorePair::new(-2, 19),
    ScorePair::new(5, 25),
    ScorePair::new(-1, 15),
    ScorePair::new(0, 8),
];
/// King danger by bucketed attack units, and safe checks by checking piece,
/// zero until fitted.
///
/// The first series measured a hand-set non-linear attacker-count term at
/// -14 Elo. This is a curve the fit shapes, and it may shape it to nothing.
const KING_DANGER_BY_BUCKET: [ScorePair; KING_DANGER_BUCKETS] = [
    ScorePair::new(0, -1),
    ScorePair::new(-3, -10),
    ScorePair::new(-7, -4),
    ScorePair::new(-3, -8),
    ScorePair::new(-2, -8),
    ScorePair::new(1, -7),
    ScorePair::new(4, 1),
    ScorePair::new(4, 2),
    ScorePair::new(2, 1),
    ScorePair::new(1, 1),
    ScorePair::new(0, 0),
    ScorePair::new(0, 0),
    ScorePair::new(0, 0),
    ScorePair::new(0, 0),
    ScorePair::new(0, 0),
    ScorePair::new(0, 0),
];
const SAFE_CHECK_BY_PIECE: [ScorePair; 4] = [
    ScorePair::new(17, 0),
    ScorePair::new(12, 12),
    ScorePair::new(19, 11),
    ScorePair::new(40, 16),
];
/// Shelter graded by the nearest pawn's distance on each file, zero until
/// fitted; the shelter count above stays as it was.
const SHELTER_KING_FILE_BY_DISTANCE: [ScorePair; 6] = [
    ScorePair::new(10, -3),
    ScorePair::new(0, 0),
    ScorePair::new(-2, -6),
    ScorePair::new(1, -1),
    ScorePair::new(0, -1),
    ScorePair::new(0, 0),
];
const SHELTER_ADJACENT_FILE_BY_DISTANCE: [ScorePair; 6] = [
    ScorePair::new(-2, -2),
    ScorePair::new(-7, 1),
    ScorePair::new(1, -3),
    ScorePair::new(0, -3),
    ScorePair::new(0, -2),
    ScorePair::new(0, 0),
];
const KING_PRESSURE: ScorePair = ScorePair::new(9, 2);
const PAWN_STORM: ScorePair = ScorePair::new(7, 1);
const THREAT: ScorePair = ScorePair::new(11, 7);
const SPACE: ScorePair = ScorePair::new(2, 0);
const PASSER_URGENCY: ScorePair = ScorePair::new(3, 6);
const COORDINATION: ScorePair = ScorePair::new(16, 2);
const SUPPORTED_THREAT: ScorePair = ScorePair::new(18, 5);
const OPEN_LINE: ScorePair = ScorePair::new(14, 1);
const PAWN_BREAK: ScorePair = ScorePair::new(12, 1);

pub(super) fn score(features: &EvalFeatures) -> ScorePair {
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
        + features.structure_indexed
        + KING_SHELTER * features.king_shelter
        + OPEN_KING_FILE * features.open_king_files
        + ROOK_OPEN_FILE * features.rook_open_files
        + ROOK_SEMI_OPEN_FILE * features.rook_semi_open_files
        + ROOK_ON_SEVENTH * features.rooks_on_seventh
        + KNIGHT_OUTPOST * features.knight_outposts
        + BISHOP_OUTPOST * features.bishop_outposts
        + BACKWARD_PAWN * features.backward_pawns
        + THREAT_MINOR_BY_PAWN * features.threat_minor_by_pawn
        + THREAT_HANGING * features.threat_hanging
        + THREAT_BY_LOWER_VALUE * features.threat_by_lower_value
        + features.piece_indexed
}

/// Weights every rank- or distance-indexed structure block at once.
///
/// Called on a structure-cache miss, so the sixty multiply-adds happen once
/// per pawn-and-king configuration rather than once per node.
pub(super) fn structure_indexed(counts: &StructureCounts) -> ScorePair {
    indexed(&PASSED_PAWN_BY_RANK, counts.passed_by_rank)
        + indexed(
            &PROTECTED_PASSED_PAWN_BY_RANK,
            counts.protected_passer_by_rank,
        )
        + indexed(&CONNECTED_PAWN_BY_RANK, counts.connected_by_rank)
        + indexed(&PASSER_OWN_KING_DISTANCE, counts.passer_own_king_distance)
        + indexed(
            &PASSER_ENEMY_KING_DISTANCE,
            counts.passer_enemy_king_distance,
        )
        + indexed(
            &SHELTER_KING_FILE_BY_DISTANCE,
            counts.shelter_king_file_by_distance,
        )
        + indexed(
            &SHELTER_ADJACENT_FILE_BY_DISTANCE,
            counts.shelter_adjacent_file_by_distance,
        )
}

/// Weight of one blockaded passer on the given rank index.
#[inline(always)]
pub(super) fn blocked_passer_weight(rank: usize) -> ScorePair {
    BLOCKED_PASSER_BY_RANK[rank]
}

/// Weight of an attack landing in the given king-danger bucket.
#[inline(always)]
pub(super) fn king_danger_weight(bucket: usize) -> ScorePair {
    KING_DANGER_BY_BUCKET[bucket]
}

/// Weight of one safe checking square for the given piece slot.
#[inline(always)]
pub(super) fn safe_check_weight(slot: usize) -> ScorePair {
    SAFE_CHECK_BY_PIECE[slot]
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
#[inline(always)]
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
#[inline(always)]
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

pub(super) fn profile_mobility_adjustment(features: &EvalFeatures) -> ScorePair {
    PAWN_MOBILITY_ADJUSTMENT * features.pawn_mobility
        + KNIGHT_MOBILITY_ADJUSTMENT * features.knight_mobility
        + BISHOP_MOBILITY_ADJUSTMENT * features.bishop_mobility
        + ROOK_MOBILITY_ADJUSTMENT * features.rook_mobility
        + QUEEN_MOBILITY_ADJUSTMENT * features.queen_mobility
        + KING_MOBILITY_ADJUSTMENT * features.king_mobility
}

pub(super) fn attacking_style(features: &EvalFeatures) -> ScorePair {
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
        &KING_DANGER_BY_BUCKET[..],
        &SAFE_CHECK_BY_PIECE[..],
        &SHELTER_KING_FILE_BY_DISTANCE[..],
        &SHELTER_ADJACENT_FILE_BY_DISTANCE[..],
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
        THREAT_MINOR_BY_PAWN,
        THREAT_HANGING,
        THREAT_BY_LOWER_VALUE,
    ]
}
