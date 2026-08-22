use super::{EvalFeatures, ScorePair};

const PAWN: ScorePair = ScorePair::new(100, 120);
const KNIGHT: ScorePair = ScorePair::new(320, 300);
const BISHOP: ScorePair = ScorePair::new(330, 320);
const ROOK: ScorePair = ScorePair::new(500, 520);
const QUEEN: ScorePair = ScorePair::new(900, 900);
const ACTIVITY: ScorePair = ScorePair::new(4, 2);
const MOBILITY: ScorePair = ScorePair::new(3, 2);
const BISHOP_PAIR: ScorePair = ScorePair::new(30, 45);
const DOUBLED_PAWN: ScorePair = ScorePair::new(-14, -18);
const ISOLATED_PAWN: ScorePair = ScorePair::new(-12, -10);
const PASSED_PAWN: ScorePair = ScorePair::new(18, 40);
const KING_SHELTER: ScorePair = ScorePair::new(10, 0);
const OPEN_KING_FILE: ScorePair = ScorePair::new(-18, -4);
const KING_PRESSURE: ScorePair = ScorePair::new(9, 2);
const INITIATIVE: ScorePair = ScorePair::new(12, 4);
const PAWN_STORM: ScorePair = ScorePair::new(7, 1);
const THREAT: ScorePair = ScorePair::new(11, 7);
const SPACE: ScorePair = ScorePair::new(2, 0);
const PASSER_URGENCY: ScorePair = ScorePair::new(3, 6);

pub(super) fn score(features: EvalFeatures) -> ScorePair {
    PAWN * features.pawns
        + KNIGHT * features.knights
        + BISHOP * features.bishops
        + ROOK * features.rooks
        + QUEEN * features.queens
        + ACTIVITY * features.activity
        + MOBILITY * features.mobility
        + BISHOP_PAIR * features.bishop_pair
        + DOUBLED_PAWN * features.doubled_pawns
        + ISOLATED_PAWN * features.isolated_pawns
        + PASSED_PAWN * features.passed_pawns
        + KING_SHELTER * features.king_shelter
        + OPEN_KING_FILE * features.open_king_files
}

pub(super) fn attacking_style(features: EvalFeatures) -> ScorePair {
    KING_PRESSURE * features.king_pressure
        + INITIATIVE * features.initiative
        + PAWN_STORM * features.pawn_storm
        + THREAT * features.threats
        + SPACE * features.space
        + PASSER_URGENCY * features.passed_pawns
}
