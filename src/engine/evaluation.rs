mod features;
mod placement;
mod tactics;
#[cfg(feature = "tuning")]
pub mod tuning;
mod weights;

pub(super) use tactics::{
    TacticalSnapshot, exchange_outcome, exchange_risk_on, style_snapshot, tactical_snapshot,
};

use std::ops::{Add, Mul};

use cozy_chess::{Board, Color, Piece};

pub(super) type Score = i32;

pub(super) const NEG_INFINITY: Score = -32_000;
pub(super) const POS_INFINITY: Score = 32_000;
pub(super) const MATE_SCORE: Score = 30_000;
pub(super) const MAX_PLY: u32 = 128;
pub(super) const MATE_THRESHOLD: Score = MATE_SCORE - MAX_PLY as Score;
pub(super) const MIN_AGGRESSION: u8 = 0;
pub(super) const DEFAULT_AGGRESSION: u8 = 75;
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

    #[cfg(any(test, feature = "tuning"))]
    pub(super) const fn middle_game(self) -> Score {
        self.middle_game
    }

    #[cfg(any(test, feature = "tuning"))]
    pub(super) const fn end_game(self) -> Score {
        self.end_game
    }

    fn scaled(self, percent: u8) -> Self {
        let percent = Score::from(percent);
        Self::new(
            self.middle_game * percent / 100,
            self.end_game * percent / 100,
        )
    }

    fn soft_bounded(self, middle_game: Score, end_game: Score) -> Self {
        Self::new(
            soft_bound(self.middle_game, middle_game),
            soft_bound(self.end_game, end_game),
        )
    }
}
fn soft_bound(score: Score, limit: Score) -> Score {
    if limit == 0 {
        return 0;
    }
    let score = i64::from(score);
    let limit = i64::from(limit);
    (score * limit / (score.abs() + limit)) as Score
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
    mobility_profile: u8,
}

impl EvaluationConfig {
    pub(super) const fn new(aggression: u8) -> Self {
        let aggression = if aggression > MAX_AGGRESSION {
            MAX_AGGRESSION
        } else {
            aggression
        };
        Self {
            aggression,
            mobility_profile: aggression,
        }
    }

    pub(super) const fn aggression(self) -> u8 {
        self.aggression
    }

    /// Returns objective scoring while retaining the selected mobility profile.
    pub(super) const fn objective_scoring(self) -> Self {
        Self {
            aggression: MIN_AGGRESSION,
            mobility_profile: self.mobility_profile,
        }
    }

    pub(super) const fn max_check_extensions(self) -> u8 {
        2 + self.aggression / 50
    }

    pub(super) const fn quiescence_check_budget(self) -> u8 {
        1 + self.aggression / 50
    }

    /// Peaks at the default profile and fades to zero at both endpoint profiles.
    pub(super) const fn mobility_profile_intensity(self) -> u8 {
        if self.mobility_profile <= DEFAULT_AGGRESSION {
            (self.mobility_profile as u16 * 100 / DEFAULT_AGGRESSION as u16) as u8
        } else {
            (MAX_AGGRESSION - self.mobility_profile) * 4
        }
    }

    pub(super) const fn root_style_margin(self) -> Score {
        let aggression = self.aggression as Score;
        aggression * aggression * 120 / 10_000
    }

    pub(super) const fn style_middle_game_cap(self) -> Score {
        self.root_style_margin() * 3 / 2
    }

    pub(super) const fn style_end_game_cap(self) -> Score {
        self.root_style_margin() * 3 / 4
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

    pub(super) fn compensation_pressure(self) -> Score {
        if self.attackers < 2 {
            return 0;
        }
        self.king_pressure
            + self.coordination() * 4
            + self.supported_threats * 5
            + self.pawn_breaks * 3
    }
}

/// Distinct move counts a knight can have, zero through eight.
pub(super) const KNIGHT_MOBILITY_ENTRIES: usize = 9;
/// Distinct move counts a bishop can have, zero through thirteen.
pub(super) const BISHOP_MOBILITY_ENTRIES: usize = 14;
/// Distinct move counts a rook can have, zero through fourteen.
pub(super) const ROOK_MOBILITY_ENTRIES: usize = 15;
/// Distinct move counts a queen can have, zero through twenty-seven.
pub(super) const QUEEN_MOBILITY_ENTRIES: usize = 28;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct EvalFeatures {
    pub(super) pawns: Score,
    pub(super) knights: Score,
    pub(super) bishops: Score,
    pub(super) rooks: Score,
    pub(super) queens: Score,
    pub(super) activity: Score,
    /// Side-relative sum of tapered piece-square deltas.
    ///
    /// This term is already a middlegame and endgame pair, so it is added to the
    /// blend directly rather than multiplied by a single weight.
    pub(super) placement: ScorePair,
    pub(super) tempo: Score,
    /// Side-relative sum of every piece's move count, kings and pawns included.
    ///
    /// The objective evaluation no longer reads this directly: pawns and kings
    /// are weighted through their own counts and the four piece types through
    /// the curves below. It is kept as the total the trace and the tests
    /// reason about.
    pub(super) mobility: Score,
    /// Side-relative sum of the per-piece mobility curves.
    ///
    /// Knights, bishops, rooks and queens are scored by a table indexed by
    /// their move count rather than by one weight times the count, so a fit
    /// can say that a piece's third square is worth more than its thirteenth,
    /// or that a trapped piece costs more than a line through the origin can
    /// express. Like placement, the term is accumulated as a pair in the
    /// piece loop and added to the blend directly; the fitter expands it back
    /// into one count per piece type and move count.
    pub(super) mobility_curves: ScorePair,
    pub(super) pawn_mobility: Score,
    pub(super) knight_mobility: Score,
    pub(super) bishop_mobility: Score,
    pub(super) rook_mobility: Score,
    pub(super) queen_mobility: Score,
    pub(super) king_mobility: Score,
    pub(super) bishop_pair: Score,
    pub(super) doubled_pawns: Score,
    pub(super) isolated_pawns: Score,
    /// Passed pawns counted per rank, from the owner's side of the board.
    pub(super) passed_by_rank: [Score; 6],
    /// Passed pawns defended by a friendly pawn, counted the same way.
    pub(super) protected_passer_by_rank: [Score; 6],
    /// Passers weighted by how far they have come.
    ///
    /// Derived from [`Self::passed_by_rank`] and read only by the attacking
    /// style, which values a runner by its progress. The objective evaluation
    /// scores passers per rank instead, so it is not forced onto a straight
    /// line through the origin.
    pub(super) passed_pawns: Score,
    pub(super) king_shelter: Score,
    pub(super) open_king_files: Score,
    /// Rooks on a file with no pawn of either colour, side-relative.
    pub(super) rook_open_files: Score,
    /// Rooks on a file with enemy pawns but none of their own.
    pub(super) rook_semi_open_files: Score,
    /// Rooks on their seventh rank with the enemy king on the eighth or enemy
    /// pawns still on the seventh to attack.
    pub(super) rooks_on_seventh: Score,
    pub(super) king_pressure: Score,
    pub(super) pawn_storm: Score,
    pub(super) threats: Score,
    pub(super) space: Score,
    pub(super) coordination: Score,
    pub(super) supported_threats: Score,
    pub(super) open_lines: Score,
    pub(super) pawn_breaks: Score,
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
    pub(super) style_middle_game_cap: Score,
    pub(super) style_end_game_cap: Score,
    pub(super) phase: Score,
    pub(super) aggression: u8,
    pub(super) blended: Score,
}

#[cfg(test)]
pub(super) fn evaluate(board: &Board) -> Score {
    evaluate_with_config(board, EvaluationConfig::default())
}

pub(super) fn evaluate_with_config(board: &Board, config: EvaluationConfig) -> Score {
    let blended = if config.aggression() == MIN_AGGRESSION {
        objective_blended_score(board, config)
    } else {
        evaluate_with_trace_and_config(board, config).blended
    };
    let relative = match board.side_to_move() {
        Color::White => blended,
        Color::Black => -blended,
    };
    debug_assert!(relative > NEG_INFINITY && relative < POS_INFINITY);
    relative
}

/// Blends the objective score without extracting style-only attack features.
///
/// Search always scores through [`EvaluationConfig::objective_scoring`], which
/// zeroes aggression and therefore scales every attacking-style weight to zero.
/// Extracting those features would compute king-pressure, threat, space, and
/// supported-threat terms only to multiply them away.
fn objective_blended_score(board: &Board, config: EvaluationConfig) -> Score {
    let features = features::extract_with_style(board, false);
    let base = weights::score(features)
        + weights::profile_mobility_adjustment(features)
            .scaled(config.mobility_profile_intensity());
    let phase = features::phase(board);
    (base.middle_game * phase + base.end_game * (24 - phase)) / 24
}
pub(super) fn root_complexity_bonus(
    board: &Board,
    mover: Color,
    config: EvaluationConfig,
) -> Score {
    if config.aggression() == 0 {
        return 0;
    }
    let snapshot = style_snapshot(board, mover);
    let forcing = snapshot.king_pressure_advantage
        + snapshot.pawn_storm_advantage
        + snapshot.threat_advantage * 2;
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
    let base = weights::score(features)
        + weights::profile_mobility_adjustment(features)
            .scaled(config.mobility_profile_intensity());
    let style = weights::attacking_style(features)
        .scaled(config.aggression())
        .soft_bounded(config.style_middle_game_cap(), config.style_end_game_cap());
    let score = base + style;
    let phase = features::phase(board);
    let blended = (score.middle_game * phase + score.end_game * (24 - phase)) / 24;

    EvaluationTrace {
        features,
        middle_game: base.middle_game,
        end_game: base.end_game,
        style_middle_game: style.middle_game,
        style_end_game: style.end_game,
        style_middle_game_cap: config.style_middle_game_cap(),
        style_end_game_cap: config.style_end_game_cap(),
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
        DEFAULT_AGGRESSION, EvalFeatures, EvaluationConfig, MATE_THRESHOLD, MAX_AGGRESSION,
        MIN_AGGRESSION, Score, evaluate, evaluate_with_config, evaluate_with_trace,
        evaluate_with_trace_and_config, root_complexity_bonus, weights,
    };
    use crate::engine::Position;
    use cozy_chess::{Color, Piece};

    #[test]
    fn objective_fast_path_matches_the_general_evaluation() {
        let fens = [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "r1bq1rk1/ppp2ppp/2n2n2/2b1p3/2B1P3/2NP1N2/PPP2PPP/R1BQ1RK1 w - - 0 8",
            "r1bqk2r/pp2bppp/2n1pn2/3p4/3P4/2NBPN2/PP3PPP/R1BQK2R b KQkq - 4 8",
            "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
            "6k1/5ppp/8/7Q/2B5/8/5PPP/6K1 w - - 0 1",
            "6k1/5ppp/8/2b5/7q/8/5PPP/6K1 b - - 0 1",
            "7k/8/8/8/8/8/8/K7 w - - 0 1",
        ];

        for profile in [MIN_AGGRESSION, 50, DEFAULT_AGGRESSION, MAX_AGGRESSION] {
            let objective = EvaluationConfig::new(profile).objective_scoring();
            for fen in fens {
                let position = Position::from_fen(fen).unwrap();
                let board = position.board();
                let general = evaluate_with_trace_and_config(board, objective).blended;

                assert_eq!(
                    super::objective_blended_score(board, objective),
                    general,
                    "profile {profile} disagreed on {fen}",
                );
            }
        }
    }

    /// Checks that skipping style extraction preserves every other feature.
    ///
    /// The style-free path is only sound for configurations that weight the
    /// attacking terms at zero, so this pins two things: the shared features are
    /// identical, and the style-only features really are absent rather than
    /// merely small. The previous form asserted that the styled aggregate was
    /// positive, which the tempo term satisfied on its own in both paths and so
    /// proved nothing about style extraction.
    #[test]
    fn style_free_extraction_preserves_material_and_mobility_features() {
        let position = Position::from_fen(
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        )
        .unwrap();
        let styled = super::features::extract_with_style(position.board(), true);
        let plain = super::features::extract_with_style(position.board(), false);

        assert_eq!(plain.mobility, styled.mobility);
        assert_eq!(plain.knight_mobility, styled.knight_mobility);
        assert_eq!(plain.bishop_mobility, styled.bishop_mobility);
        assert_eq!(plain.rook_mobility, styled.rook_mobility);
        assert_eq!(plain.queen_mobility, styled.queen_mobility);
        assert_eq!(plain.pawn_mobility, styled.pawn_mobility);
        assert_eq!(plain.king_mobility, styled.king_mobility);
        assert_eq!(plain.passed_pawns, styled.passed_pawns);
        assert_eq!(plain.king_shelter, styled.king_shelter);
        assert_eq!(plain.placement, styled.placement);
        assert_eq!(plain.tempo, styled.tempo);
        assert_eq!(weights::score(plain), weights::score(styled));
        assert_eq!(
            weights::profile_mobility_adjustment(plain),
            weights::profile_mobility_adjustment(styled),
        );

        for (name, value) in [
            ("king_pressure", plain.king_pressure),
            ("threats", plain.threats),
            ("space", plain.space),
            ("coordination", plain.coordination),
            ("supported_threats", plain.supported_threats),
            ("open_lines", plain.open_lines),
            ("pawn_breaks", plain.pawn_breaks),
            ("pawn_storm", plain.pawn_storm),
        ] {
            assert_eq!(value, 0, "style-free extraction produced {name}");
        }
        assert_ne!(styled.king_pressure, 0);

        // A position with a live attack shows the style bucket is genuinely fed.
        let attacker = Position::from_fen("6k1/5ppp/8/7Q/2B5/8/5PPP/6K1 w - - 0 1").unwrap();
        let attacking = super::features::extract(attacker.board());
        assert!(weights::attacking_style(attacking).middle_game > 0);
    }

    /// The starting position is symmetric apart from whose turn it is.
    ///
    /// Material, placement, and pawn structure all cancel, so the whole score is
    /// the tempo bonus for the side to move. That makes this the test that pins
    /// tempo as a personality-neutral term rather than a style one.
    #[test]
    fn the_starting_position_scores_only_a_tempo() {
        let white = Position::default();
        let black =
            Position::from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR b KQkq - 0 1").unwrap();
        let base = EvaluationConfig::new(MIN_AGGRESSION);
        let trace = evaluate_with_trace_and_config(white.board(), base);

        assert_eq!(trace.features.placement, super::ScorePair::default());
        assert_eq!(trace.features.tempo, 1);
        let score = evaluate_with_config(white.board(), base);
        assert!(score > 0, "the side to move holds a tempo, scored {score}");
        assert_eq!(evaluate_with_config(black.board(), base), score);
    }

    #[test]
    fn material_is_scored_for_the_side_to_move() {
        let white = Position::from_fen("7k/8/8/8/8/8/8/3QK3 w - - 0 1").unwrap();
        let black = Position::from_fen("7k/8/8/8/8/8/8/3QK3 b - - 0 1").unwrap();
        let base = EvaluationConfig::new(MIN_AGGRESSION);

        // Both positions are the same placement with the turn changed, so the two
        // scores differ by twice the tempo rather than being exact negations.
        let white_score = evaluate_with_config(white.board(), base);
        let black_score = evaluate_with_config(black.board(), base);
        assert!(white_score > 900);
        assert!(black_score < -900);
        assert!(
            white_score > -black_score,
            "the side to move should not lose its tempo: {white_score} against {black_score}",
        );
    }

    #[test]
    fn material_evaluation_does_not_embed_terminal_scores() {
        let position = Position::from_fen("7k/6Q1/6K1/8/8/8/8/8 b - - 0 1").unwrap();

        let score = evaluate(position.board());
        assert!(score < 0);
        assert!(score.abs() < MATE_THRESHOLD);
    }

    /// A bare-kings position is materially dead but not positionally identical.
    ///
    /// The kings stand on different squares, so the endgame king table gives one
    /// side a small placement edge; the two-king ending is recognized as drawn by
    /// search rather than by evaluation. What must hold is that the score stays
    /// far below anything decisive.
    #[test]
    fn drawn_positions_score_near_zero_without_style() {
        let position = Position::from_fen("7k/8/8/8/8/8/8/K7 w - - 0 1").unwrap();

        let score = evaluate_with_config(position.board(), EvaluationConfig::new(MIN_AGGRESSION));

        assert!(
            score.abs() < 100,
            "a dead position should not look decisive, scored {score}",
        );
    }

    /// Placement is mirrored between colours, so a mirrored position is scored
    /// identically for whichever side is to move.
    #[test]
    fn placement_is_symmetric_between_mirrored_positions() {
        let white = Position::from_fen("6k1/5ppp/8/8/8/8/5PPP/6K1 w - - 0 1").unwrap();
        let black = Position::from_fen("6k1/5ppp/8/8/8/8/5PPP/6K1 b - - 0 1").unwrap();
        let base = EvaluationConfig::new(MIN_AGGRESSION);

        assert_eq!(
            evaluate_with_config(white.board(), base),
            evaluate_with_config(black.board(), base),
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
    fn piece_mobility_is_weighted_without_entering_style() {
        let knight = Position::from_fen("4k3/8/8/8/3N4/8/8/4K3 w - - 0 1").unwrap();
        let bishop = Position::from_fen("4k3/8/8/8/3B4/8/8/4K3 w - - 0 1").unwrap();
        let rook = Position::from_fen("4k3/8/8/8/3R4/8/8/4K3 w - - 0 1").unwrap();
        let queen = Position::from_fen("4k3/8/8/8/3Q4/8/8/4K3 w - - 0 1").unwrap();

        assert_eq!(
            evaluate_with_trace(knight.board()).features.knight_mobility,
            8
        );
        assert_eq!(
            evaluate_with_trace(bishop.board()).features.bishop_mobility,
            13
        );
        assert_eq!(evaluate_with_trace(rook.board()).features.rook_mobility, 14);
        assert_eq!(
            evaluate_with_trace(queen.board()).features.queen_mobility,
            27
        );

        let generic = EvalFeatures {
            pawn_mobility: 10,
            ..EvalFeatures::default()
        };
        let pieces = EvalFeatures {
            knight_mobility: 1,
            bishop_mobility: 1,
            rook_mobility: 1,
            queen_mobility: 1,
            ..EvalFeatures::default()
        };
        assert_ne!(
            super::weights::score(generic),
            super::weights::score(EvalFeatures::default())
        );
        assert_eq!(
            super::weights::score(pieces),
            super::weights::score(EvalFeatures::default())
        );
        assert_ne!(
            super::weights::profile_mobility_adjustment(pieces),
            super::weights::profile_mobility_adjustment(EvalFeatures::default())
        );
        assert_eq!(
            super::weights::attacking_style(pieces),
            super::weights::attacking_style(EvalFeatures::default())
        );

        let features = evaluate_with_trace(queen.board()).features;
        assert_eq!(
            features.mobility,
            features.pawn_mobility
                + features.knight_mobility
                + features.bishop_mobility
                + features.rook_mobility
                + features.queen_mobility
                + features.king_mobility
        );
        let profiled = super::ScorePair::new(3, 2) * features.mobility
            + super::weights::profile_mobility_adjustment(features);
        let explicit = super::ScorePair::new(4, 4) * features.knight_mobility
            + super::ScorePair::new(5, 5) * features.bishop_mobility
            + super::ScorePair::new(2, 4) * features.rook_mobility
            + super::ScorePair::new(1, 2) * features.queen_mobility;
        assert_eq!(profiled, explicit);
    }

    /// The per-piece curves ship as the linear term they replaced.
    ///
    /// Until a fit moves them, a knight with five moves must score exactly
    /// what five units of the shared mobility weight score, for every count
    /// of every piece, which is what makes adopting the curves a change to no
    /// score at all.
    #[test]
    fn mobility_curves_start_on_the_linear_term() {
        let unit = super::weights::score(EvalFeatures {
            pawn_mobility: 1,
            ..EvalFeatures::default()
        });
        for (piece, entries) in [
            (Piece::Knight, super::KNIGHT_MOBILITY_ENTRIES),
            (Piece::Bishop, super::BISHOP_MOBILITY_ENTRIES),
            (Piece::Rook, super::ROOK_MOBILITY_ENTRIES),
            (Piece::Queen, super::QUEEN_MOBILITY_ENTRIES),
        ] {
            for count in 0..entries {
                assert_eq!(
                    super::weights::mobility_curve(piece, count),
                    unit * count as Score,
                    "{piece:?} with {count} moves left the linear term",
                );
            }
        }
        for piece in [Piece::Pawn, Piece::King] {
            for count in 0..super::QUEEN_MOBILITY_ENTRIES {
                assert_eq!(
                    super::weights::mobility_curve(piece, count),
                    super::ScorePair::new(0, 0)
                );
            }
        }

        // In a real position the accumulated pair says what the per-piece
        // counts say, and the objective score reads it.
        let position = Position::from_fen(
            "r1bqkb1r/pppp1ppp/2n2n2/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4",
        )
        .unwrap();
        let features = evaluate_with_trace(position.board()).features;
        let curved = features.knight_mobility
            + features.bishop_mobility
            + features.rook_mobility
            + features.queen_mobility;
        assert_eq!(features.mobility_curves, unit * curved);
        assert_eq!(
            features.mobility,
            curved + features.pawn_mobility + features.king_mobility
        );
        let without = EvalFeatures {
            mobility_curves: super::ScorePair::new(0, 0),
            ..features
        };
        assert_eq!(
            super::weights::score(features),
            super::weights::score(without) + features.mobility_curves
        );
    }

    #[test]
    fn rook_files_and_the_seventh_are_counted_for_each_side() {
        let features =
            |fen: &str| evaluate_with_trace(Position::from_fen(fen).unwrap().board()).features;

        let open = features("4k3/8/8/8/8/8/4P3/R3K3 w - - 0 1");
        assert_eq!(open.rook_open_files, 1);
        assert_eq!(open.rook_semi_open_files, 0);

        let semi_open = features("4k3/p7/8/8/8/8/4P3/R3K3 w - - 0 1");
        assert_eq!(semi_open.rook_open_files, 0);
        assert_eq!(semi_open.rook_semi_open_files, 1);

        let closed = features("4k3/p7/8/8/8/8/P7/R3K3 w - - 0 1");
        assert_eq!(closed.rook_open_files, 0);
        assert_eq!(closed.rook_semi_open_files, 0);

        // The seventh counts against a king on the eighth or pawns to attack,
        // and not otherwise.
        assert_eq!(
            features("4k3/R7/8/8/8/8/8/4K3 w - - 0 1").rooks_on_seventh,
            1
        );
        assert_eq!(
            features("8/R6p/4k3/8/8/8/8/4K3 w - - 0 1").rooks_on_seventh,
            1
        );
        assert_eq!(
            features("8/R7/4k3/8/8/8/8/4K3 w - - 0 1").rooks_on_seventh,
            0
        );
        // Black's seventh is the second rank, counted with the opposite sign.
        assert_eq!(
            features("4k3/8/8/8/8/8/r7/4K3 w - - 0 1").rooks_on_seventh,
            -1
        );
        assert_eq!(
            features("4k3/8/8/8/8/8/P6r/R3K3 w - - 0 1").rook_open_files,
            -1
        );
        assert_eq!(
            features("4k3/8/8/8/8/7P/P6r/R3K3 w - - 0 1").rook_semi_open_files,
            -1
        );

        // The weights ship at zero, so none of this moves a score yet.
        let scored = EvalFeatures {
            rook_open_files: 1,
            rook_semi_open_files: 1,
            rooks_on_seventh: 1,
            ..EvalFeatures::default()
        };
        assert_eq!(
            super::weights::score(scored),
            super::weights::score(EvalFeatures::default())
        );
    }

    #[test]
    fn aggression_is_clamped_and_scales_tempered_style_caps() {
        let position = Position::from_fen("6k1/5ppp/8/7Q/2B5/8/5PPP/6K1 w - - 0 1").unwrap();
        let quiet =
            evaluate_with_trace_and_config(position.board(), EvaluationConfig::new(MIN_AGGRESSION));
        let midpoint = evaluate_with_trace_and_config(position.board(), EvaluationConfig::new(50));
        let aggressive =
            evaluate_with_trace_and_config(position.board(), EvaluationConfig::new(MAX_AGGRESSION));
        let clamped =
            evaluate_with_trace_and_config(position.board(), EvaluationConfig::new(u8::MAX));

        assert_eq!(EvaluationConfig::default().aggression(), DEFAULT_AGGRESSION);
        assert_eq!(
            EvaluationConfig::new(MIN_AGGRESSION).mobility_profile_intensity(),
            0
        );
        assert_eq!(
            EvaluationConfig::default().mobility_profile_intensity(),
            100
        );
        let objective = EvaluationConfig::default().objective_scoring();
        assert_eq!(objective.aggression(), MIN_AGGRESSION);
        assert_eq!(objective.mobility_profile_intensity(), 100);
        assert_eq!(
            EvaluationConfig::new(MAX_AGGRESSION).mobility_profile_intensity(),
            0
        );
        assert_eq!(clamped.aggression, MAX_AGGRESSION);
        assert_eq!(
            (quiet.style_middle_game_cap, quiet.style_end_game_cap),
            (0, 0)
        );
        assert_eq!(quiet.style_middle_game, 0);
        assert_eq!(quiet.style_end_game, 0);
        assert_eq!(
            (midpoint.style_middle_game_cap, midpoint.style_end_game_cap),
            (45, 22),
        );
        assert_eq!(
            (
                aggressive.style_middle_game_cap,
                aggressive.style_end_game_cap,
            ),
            (180, 90),
        );
        assert!(aggressive.style_middle_game > 0);
        assert!(aggressive.style_middle_game.abs() <= aggressive.style_middle_game_cap);
        assert!(aggressive.style_end_game.abs() <= aggressive.style_end_game_cap);
        assert_eq!(aggressive, clamped);
        assert!(aggressive.blended > quiet.blended);
    }
    #[test]
    fn soft_style_bound_preserves_sign_and_order_below_its_limit() {
        assert_eq!(super::soft_bound(0, 120), 0);
        assert_eq!(super::soft_bound(80, 0), 0);
        assert!(super::soft_bound(300, 120) > super::soft_bound(100, 120));
        assert!(super::soft_bound(-300, 120) < super::soft_bound(-100, 120));
        assert!(super::soft_bound(10_000, 120).abs() < 120);
        assert!(super::soft_bound(-10_000, 120).abs() < 120);
    }

    #[test]
    fn coordinated_attack_keeps_more_raw_style_than_a_lone_attacker() {
        let lone = Position::from_fen("6k1/5ppp/8/7Q/8/8/5PPP/6K1 w - - 0 1").unwrap();
        let coordinated = Position::from_fen("6k1/5ppp/8/7Q/2B5/8/5PPP/6K1 w - - 0 1").unwrap();
        let config = EvaluationConfig::new(MAX_AGGRESSION);
        let lone_trace = evaluate_with_trace_and_config(lone.board(), config);
        let coordinated_trace = evaluate_with_trace_and_config(coordinated.board(), config);

        assert_eq!(lone_trace.features.white_attack.coordination(), 0);
        assert!(coordinated_trace.features.white_attack.coordination() > 0);
        assert!(
            weights::attacking_style(coordinated_trace.features).middle_game
                > weights::attacking_style(lone_trace.features).middle_game
        );
        assert!(coordinated_trace.style_middle_game >= lone_trace.style_middle_game);
    }

    #[test]
    fn attacking_style_does_not_refund_a_material_deficit() {
        let pressure = EvalFeatures {
            king_pressure: 40,
            coordination: 2,
            supported_threats: 1,
            ..EvalFeatures::default()
        };
        let deficit = EvalFeatures {
            pawns: -3,
            rooks: -1,
            ..pressure
        };

        assert_eq!(
            weights::attacking_style(pressure),
            weights::attacking_style(deficit)
        );
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
