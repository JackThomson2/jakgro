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
pub(super) const MIN_AGGRESSION: u8 = 0;
pub(super) const DEFAULT_AGGRESSION: u8 = 100;
pub(super) const MAX_AGGRESSION: u8 = 100;

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

    fn scaled(self, percent: u8) -> Self {
        let percent = Score::from(percent);
        Self::new(
            self.middle_game * percent / 100,
            self.end_game * percent / 100,
        )
    }

    fn clamped(self, middle_game: Score, end_game: Score) -> Self {
        Self::new(
            self.middle_game.clamp(-middle_game, middle_game),
            self.end_game.clamp(-end_game, end_game),
        )
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct EvaluationConfig {
    aggression: u8,
}

impl EvaluationConfig {
    pub(super) const fn new(aggression: u8) -> Self {
        Self {
            aggression: if aggression > MAX_AGGRESSION {
                MAX_AGGRESSION
            } else {
                aggression
            },
        }
    }

    pub(super) const fn aggression(self) -> u8 {
        self.aggression
    }

    pub(super) const fn max_check_extensions(self) -> u8 {
        2 + self.aggression / 50
    }

    pub(super) const fn quiescence_check_budget(self) -> u8 {
        1 + self.aggression / 50
    }

    pub(super) const fn root_style_margin(self) -> Score {
        let aggression = self.aggression as Score;
        aggression * aggression * 120 / 10_000
    }
}

impl Default for EvaluationConfig {
    fn default() -> Self {
        Self::new(DEFAULT_AGGRESSION)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct AttackProfile {
    pub(super) king_pressure: Score,
    pub(super) attackers: Score,
    pub(super) attacker_variety: Score,
    pub(super) supported_threats: Score,
    pub(super) open_lines: Score,
    pub(super) pawn_breaks: Score,
    pub(super) pawn_storm: Score,
    pub(super) threats: Score,
    pub(super) space: Score,
    pub(super) defender_shortage: Score,
}

impl AttackProfile {
    pub(super) fn coordination(self) -> Score {
        if self.attackers < 2 {
            return 0;
        }
        (self.attackers - 1) * 3
            + self.attacker_variety * 2
            + self.open_lines * 2
            + self.supported_threats
            + self.defender_shortage * 2
    }

    fn compensation_pressure(self) -> Score {
        if self.attackers < 2 {
            return 0;
        }
        self.king_pressure
            + self.coordination() * 4
            + self.supported_threats * 5
            + self.pawn_breaks * 3
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
    pub(super) king_pressure: Score,
    pub(super) initiative: Score,
    pub(super) pawn_storm: Score,
    pub(super) threats: Score,
    pub(super) space: Score,
    pub(super) coordination: Score,
    pub(super) supported_threats: Score,
    pub(super) open_lines: Score,
    pub(super) pawn_breaks: Score,
    pub(super) compensation: Score,
    pub(super) white_attack: AttackProfile,
    pub(super) black_attack: AttackProfile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct EvaluationTrace {
    pub(super) features: EvalFeatures,
    pub(super) middle_game: Score,
    pub(super) end_game: Score,
    pub(super) style_middle_game: Score,
    pub(super) style_end_game: Score,
    pub(super) phase: Score,
    pub(super) aggression: u8,
    pub(super) blended: Score,
}

#[cfg(test)]
pub(super) fn evaluate(board: &Board) -> Score {
    evaluate_with_config(board, EvaluationConfig::default())
}

pub(super) fn evaluate_with_config(board: &Board, config: EvaluationConfig) -> Score {
    let trace = evaluate_with_trace_and_config(board, config);
    let relative = match board.side_to_move() {
        Color::White => trace.blended,
        Color::Black => -trace.blended,
    };
    debug_assert!(relative > NEG_INFINITY && relative < POS_INFINITY);
    relative
}
pub(super) fn root_complexity_bonus(
    board: &Board,
    mover: Color,
    config: EvaluationConfig,
) -> Score {
    if config.aggression() == 0 {
        return 0;
    }
    let features = features::extract(board);
    let sign = if mover == Color::White { 1 } else { -1 };
    let forcing = sign * (features.king_pressure + features.pawn_storm + features.threats * 2);
    (forcing.max(0) / 4).min(12) * Score::from(config.aggression()) / 100
}

#[cfg(test)]
pub(super) fn evaluate_with_trace(board: &Board) -> EvaluationTrace {
    evaluate_with_trace_and_config(board, EvaluationConfig::default())
}

pub(super) fn evaluate_with_trace_and_config(
    board: &Board,
    config: EvaluationConfig,
) -> EvaluationTrace {
    let features = features::extract(board);
    let base = weights::score(features);
    let style = weights::attacking_style(features)
        .clamped(450, 220)
        .scaled(config.aggression());
    let score = base + style;
    let phase = features::phase(board);
    let blended = (score.middle_game * phase + score.end_game * (24 - phase)) / 24;

    EvaluationTrace {
        features,
        middle_game: base.middle_game,
        end_game: base.end_game,
        style_middle_game: style.middle_game,
        style_end_game: style.end_game,
        phase,
        aggression: config.aggression(),
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
    use super::{
        DEFAULT_AGGRESSION, EvaluationConfig, MATE_THRESHOLD, MAX_AGGRESSION, MIN_AGGRESSION,
        evaluate, evaluate_with_config, evaluate_with_trace, evaluate_with_trace_and_config,
        root_complexity_bonus,
    };
    use crate::engine::Position;
    use cozy_chess::Color;

    #[test]
    fn starting_material_is_equal_without_style() {
        assert_eq!(
            evaluate_with_config(
                Position::default().board(),
                EvaluationConfig::new(MIN_AGGRESSION),
            ),
            0
        );
    }

    #[test]
    fn material_is_scored_for_the_side_to_move() {
        let white = Position::from_fen("7k/8/8/8/8/8/8/3QK3 w - - 0 1").unwrap();
        let black = Position::from_fen("7k/8/8/8/8/8/8/3QK3 b - - 0 1").unwrap();
        let base = EvaluationConfig::new(MIN_AGGRESSION);

        let white_score = evaluate_with_config(white.board(), base);
        assert!(white_score > 900);
        assert_eq!(evaluate_with_config(black.board(), base), -white_score);
    }

    #[test]
    fn material_evaluation_does_not_embed_terminal_scores() {
        let position = Position::from_fen("7k/6Q1/6K1/8/8/8/8/8 b - - 0 1").unwrap();

        let score = evaluate(position.board());
        assert!(score < 0);
        assert!(score.abs() < MATE_THRESHOLD);
    }

    #[test]
    fn drawn_positions_are_neutral_without_style() {
        let position = Position::from_fen("7k/8/8/8/8/8/8/K7 w - - 0 1").unwrap();

        assert_eq!(
            evaluate_with_config(position.board(), EvaluationConfig::new(MIN_AGGRESSION)),
            0
        );
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
    fn aggression_is_clamped_and_scales_bounded_style_terms() {
        let position = Position::from_fen("6k1/5ppp/8/7Q/2B5/8/5PPP/6K1 w - - 0 1").unwrap();
        let quiet =
            evaluate_with_trace_and_config(position.board(), EvaluationConfig::new(MIN_AGGRESSION));
        let aggressive =
            evaluate_with_trace_and_config(position.board(), EvaluationConfig::new(MAX_AGGRESSION));
        let clamped =
            evaluate_with_trace_and_config(position.board(), EvaluationConfig::new(u8::MAX));

        assert_eq!(EvaluationConfig::default().aggression(), DEFAULT_AGGRESSION);
        assert_eq!(clamped.aggression, MAX_AGGRESSION);
        assert_eq!(quiet.style_middle_game, 0);
        assert_eq!(quiet.style_end_game, 0);
        assert!(aggressive.style_middle_game > 0);
        assert!(aggressive.style_middle_game.abs() <= 450);
        assert!(aggressive.style_end_game.abs() <= 220);
        assert_eq!(aggressive, clamped);
        assert!(aggressive.blended > quiet.blended);
    }

    #[test]
    fn coordinated_attack_scores_more_style_than_a_lone_attacker() {
        let lone = Position::from_fen("6k1/5ppp/8/7Q/8/8/5PPP/6K1 w - - 0 1").unwrap();
        let coordinated = Position::from_fen("6k1/5ppp/8/7Q/2B5/8/5PPP/6K1 w - - 0 1").unwrap();
        let config = EvaluationConfig::new(MAX_AGGRESSION);
        let lone_trace = evaluate_with_trace_and_config(lone.board(), config);
        let coordinated_trace = evaluate_with_trace_and_config(coordinated.board(), config);

        assert_eq!(lone_trace.features.white_attack.coordination(), 0);
        assert!(coordinated_trace.features.white_attack.coordination() > 0);
        assert!(coordinated_trace.style_middle_game > lone_trace.style_middle_game);
    }

    #[test]
    fn coordinated_pressure_can_compensate_a_material_deficit() {
        let position = Position::from_fen("qr4k1/5ppp/8/7Q/2B5/8/5PPP/6K1 w - - 0 1").unwrap();
        let trace =
            evaluate_with_trace_and_config(position.board(), EvaluationConfig::new(MAX_AGGRESSION));

        assert!(trace.features.compensation > 0);
    }

    #[test]
    fn attacking_style_is_color_symmetric() {
        let white = Position::from_fen("6k1/5ppp/8/7Q/2B5/8/5PPP/6K1 w - - 0 1").unwrap();
        let black = Position::from_fen("6k1/5ppp/8/2b5/7q/8/5PPP/6K1 b - - 0 1").unwrap();
        let config = EvaluationConfig::new(MAX_AGGRESSION);

        assert_eq!(
            evaluate_with_config(white.board(), config),
            evaluate_with_config(black.board(), config),
        );
    }

    #[test]
    fn root_complexity_bonus_is_scaled_and_bounded() {
        let white = Position::from_fen("6k1/5ppp/8/7Q/2B5/8/5PPP/6K1 w - - 0 1").unwrap();
        let black = Position::from_fen("6k1/5ppp/8/2b5/7q/8/5PPP/6K1 b - - 0 1").unwrap();

        assert_eq!(
            root_complexity_bonus(
                white.board(),
                Color::White,
                EvaluationConfig::new(MIN_AGGRESSION),
            ),
            0,
        );
        let white_bonus = root_complexity_bonus(
            white.board(),
            Color::White,
            EvaluationConfig::new(MAX_AGGRESSION),
        );
        let black_bonus = root_complexity_bonus(
            black.board(),
            Color::Black,
            EvaluationConfig::new(MAX_AGGRESSION),
        );
        assert!((1..=12).contains(&white_bonus));
        assert_eq!(white_bonus, black_bonus);
    }

    #[test]
    fn color_swapped_material_is_symmetric_for_the_side_to_move() {
        let white = Position::from_fen("4k3/8/8/8/8/8/Q7/4K3 w - - 0 1").unwrap();
        let black = Position::from_fen("4k3/q7/8/8/8/8/8/4K3 b - - 0 1").unwrap();

        assert_eq!(evaluate(white.board()), evaluate(black.board()));
    }
}
