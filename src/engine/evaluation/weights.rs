use super::{EvalFeatures, ScorePair};

const PAWN: ScorePair = ScorePair::new(94, 149);
const KNIGHT: ScorePair = ScorePair::new(330, 290);
const BISHOP: ScorePair = ScorePair::new(347, 327);
const ROOK: ScorePair = ScorePair::new(503, 547);
const QUEEN: ScorePair = ScorePair::new(926, 932);
const ACTIVITY: ScorePair = ScorePair::new(3, -3);
const TEMPO: ScorePair = ScorePair::new(12, 0);
const MOBILITY: ScorePair = ScorePair::new(3, 7);
const PAWN_MOBILITY_ADJUSTMENT: ScorePair = ScorePair::new(-3, -2);
const KNIGHT_MOBILITY_ADJUSTMENT: ScorePair = ScorePair::new(1, 2);
const BISHOP_MOBILITY_ADJUSTMENT: ScorePair = ScorePair::new(2, 3);
const ROOK_MOBILITY_ADJUSTMENT: ScorePair = ScorePair::new(-1, 2);
const QUEEN_MOBILITY_ADJUSTMENT: ScorePair = ScorePair::new(-2, 0);
const KING_MOBILITY_ADJUSTMENT: ScorePair = ScorePair::new(-3, -2);
const BISHOP_PAIR: ScorePair = ScorePair::new(36, 47);
const DOUBLED_PAWN: ScorePair = ScorePair::new(-17, -21);
const ISOLATED_PAWN: ScorePair = ScorePair::new(-9, -10);
const PASSED_PAWN: ScorePair = ScorePair::new(5, 16);
const KING_SHELTER: ScorePair = ScorePair::new(23, -12);
const OPEN_KING_FILE: ScorePair = ScorePair::new(-18, -3);
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
        + MOBILITY * features.mobility
        + BISHOP_PAIR * features.bishop_pair
        + DOUBLED_PAWN * features.doubled_pawns
        + ISOLATED_PAWN * features.isolated_pawns
        + PASSED_PAWN * features.passed_pawns
        + KING_SHELTER * features.king_shelter
        + OPEN_KING_FILE * features.open_king_files
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
pub(super) const fn tuning_weights() -> [ScorePair; 14] {
    [
        PAWN,
        KNIGHT,
        BISHOP,
        ROOK,
        QUEEN,
        ACTIVITY,
        TEMPO,
        MOBILITY,
        BISHOP_PAIR,
        DOUBLED_PAWN,
        ISOLATED_PAWN,
        PASSED_PAWN,
        KING_SHELTER,
        OPEN_KING_FILE,
    ]
}
