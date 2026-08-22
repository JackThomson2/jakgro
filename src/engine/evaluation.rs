mod features;
mod weights;

use std::ops::{Add, Mul};

use cozy_chess::{Board, Color, Piece};

pub(super) type Score = i32;

pub(super) const NEG_INFINITY: Score = -32_000;
pub(super) const POS_INFINITY: Score = 32_000;
pub(super) const MATE_SCORE: Score = 30_000;
pub(super) const MAX_PLY: u32 = 128;
pub(super) const MATE_THRESHOLD: Score = MATE_SCORE - MAX_PLY as Score;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ScorePair {
    middle_game: Score,
    end_game: Score,
}

impl ScorePair {
    pub(super) const fn new(middle_game: Score, end_game: Score) -> Self {
        Self {
            middle_game,
            end_game,
        }
    }
}

impl Add for ScorePair {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self::new(
            self.middle_game + other.middle_game,
            self.end_game + other.end_game,
        )
    }
}

impl Mul<Score> for ScorePair {
    type Output = Self;

    fn mul(self, feature: Score) -> Self {
        Self::new(self.middle_game * feature, self.end_game * feature)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct EvalFeatures {
    pub(super) pawns: Score,
    pub(super) knights: Score,
    pub(super) bishops: Score,
    pub(super) rooks: Score,
    pub(super) queens: Score,
    pub(super) activity: Score,
    pub(super) mobility: Score,
    pub(super) bishop_pair: Score,
    pub(super) doubled_pawns: Score,
    pub(super) isolated_pawns: Score,
    pub(super) passed_pawns: Score,
    pub(super) king_shelter: Score,
    pub(super) open_king_files: Score,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct EvaluationTrace {
    pub(super) features: EvalFeatures,
    pub(super) middle_game: Score,
    pub(super) end_game: Score,
    pub(super) phase: Score,
    pub(super) blended: Score,
}

pub(super) fn evaluate(board: &Board) -> Score {
    let trace = evaluate_with_trace(board);
    let relative = match board.side_to_move() {
        Color::White => trace.blended,
        Color::Black => -trace.blended,
    };
    debug_assert!(relative > NEG_INFINITY && relative < POS_INFINITY);
    relative
}

pub(super) fn evaluate_with_trace(board: &Board) -> EvaluationTrace {
    let features = features::extract(board);
    let score = weights::score(features);
    let phase = features::phase(board);
    let blended = (score.middle_game * phase + score.end_game * (24 - phase)) / 24;

    EvaluationTrace {
        features,
        middle_game: score.middle_game,
        end_game: score.end_game,
        phase,
        blended,
    }
}

pub(super) const fn piece_value(piece: Piece) -> Score {
    match piece {
        Piece::Pawn => 100,
        Piece::Knight => 320,
        Piece::Bishop => 330,
        Piece::Rook => 500,
        Piece::Queen => 900,
        Piece::King => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::{MATE_THRESHOLD, evaluate, evaluate_with_trace};
    use crate::engine::Position;

    #[test]
    fn starting_material_is_equal() {
        assert_eq!(evaluate(Position::default().board()), 0);
    }

    #[test]
    fn material_is_scored_for_the_side_to_move() {
        let white = Position::from_fen("7k/8/8/8/8/8/8/3QK3 w - - 0 1").unwrap();
        let black = Position::from_fen("7k/8/8/8/8/8/8/3QK3 b - - 0 1").unwrap();

        let white_score = evaluate(white.board());
        assert!(white_score > 900);
        assert_eq!(evaluate(black.board()), -white_score);
    }

    #[test]
    fn material_evaluation_does_not_embed_terminal_scores() {
        let position = Position::from_fen("7k/6Q1/6K1/8/8/8/8/8 b - - 0 1").unwrap();

        let score = evaluate(position.board());
        assert!(score < 0);
        assert!(score.abs() < MATE_THRESHOLD);
    }

    #[test]
    fn drawn_positions_are_neutral() {
        let position = Position::from_fen("7k/8/8/8/8/8/8/K7 w - - 0 1").unwrap();

        assert_eq!(evaluate(position.board()), 0);
    }
    #[test]
    fn phase_tracks_remaining_non_pawn_material() {
        let starting = evaluate_with_trace(Position::default().board());
        let kings = Position::from_fen("4k3/8/8/8/8/8/8/4K3 w - - 0 1").unwrap();

        assert_eq!(starting.phase, 24);
        assert_eq!(evaluate_with_trace(kings.board()).phase, 0);
    }

    #[test]
    fn feature_trace_exposes_pawn_structure() {
        let doubled = Position::from_fen("4k3/8/8/8/8/P7/P7/4K3 w - - 0 1").unwrap();
        let passer = Position::from_fen("4k3/8/8/4P3/8/8/8/4K3 w - - 0 1").unwrap();
        let blocked = Position::from_fen("4k3/8/4p3/4P3/8/8/8/4K3 w - - 0 1").unwrap();

        assert_eq!(
            evaluate_with_trace(doubled.board()).features.doubled_pawns,
            1
        );
        assert!(
            evaluate_with_trace(passer.board()).features.passed_pawns
                > evaluate_with_trace(blocked.board()).features.passed_pawns
        );
    }

    #[test]
    fn color_swapped_material_is_symmetric_for_the_side_to_move() {
        let white = Position::from_fen("4k3/8/8/8/8/8/Q7/4K3 w - - 0 1").unwrap();
        let black = Position::from_fen("4k3/q7/8/8/8/8/8/4K3 b - - 0 1").unwrap();

        assert_eq!(evaluate(white.board()), evaluate(black.board()));
    }
}
