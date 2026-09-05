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
/// Buckets of the king-danger table, indexed by attack units.
pub(super) const KING_DANGER_BUCKETS: usize = 16;

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
    /// Pawns no neighbour can support whose advance is not safe.
    pub(super) backward_pawns: Score,
    /// Every rank- or distance-indexed pawn and king structure block, already
    /// weighted: passers, protected passers and connected pawns by rank,
    /// passers by each king's distance, and shelter and storm by pawn
    /// distance.
    ///
    /// Like placement, this is a pair added to the blend directly. The
    /// counts behind it are a function of the pawns and the kings, so they
    /// are weighted once on a structure-cache miss rather than carried to
    /// every node; [`features::structure_counts`] recomputes them for the
    /// fitter and the tests.
    pub(super) structure_indexed: ScorePair,
    /// The indexed blocks the piece loop produces, already weighted: passers
    /// blockaded by rank, king danger by bucketed attack units, and safe
    /// checks by checking piece. A colour bringing nothing against the enemy
    /// king counts in no danger bucket: the term describes an attack, not its
    /// absence.
    pub(super) piece_indexed: ScorePair,
    /// Passers weighted by how far they have come.
    ///
    /// Derived from the per-rank counts and read only by the attacking style,
    /// which values a runner by its progress. The objective evaluation scores
    /// passers per rank instead, so it is not forced onto a straight line
    /// through the origin.
    pub(super) passed_pawns: Score,
    pub(super) king_shelter: Score,
    pub(super) open_king_files: Score,
    /// Enemy minor pieces attacked by a pawn, side-relative.
    pub(super) threat_minor_by_pawn: Score,
    /// Enemy pieces, pawns and kings aside, attacked and defended by nothing.
    pub(super) threat_hanging: Score,
    /// Enemy rooks and queens attacked by a pawn or a minor, and queens
    /// attacked by a rook.
    pub(super) threat_by_lower_value: Score,
    /// Rooks on a file with no pawn of either colour, side-relative.
    pub(super) rook_open_files: Score,
    /// Rooks on a file with enemy pawns but none of their own.
    pub(super) rook_semi_open_files: Score,
    /// Rooks on their seventh rank with the enemy king on the eighth or enemy
    /// pawns still on the seventh to attack.
    pub(super) rooks_on_seventh: Score,
    /// Minor pieces on an outpost: a square on the fourth to sixth rank from
    /// the owner's side, defended by a friendly pawn, that no enemy pawn can
    /// ever attack.
    pub(super) knight_outposts: Score,
    pub(super) bishop_outposts: Score,
    /// Safe centre squares behind the pawn chain, side-relative, and the
    /// same count scaled by the owner's pieces other than pawns and the
    /// king, since room is worth what wants to use it.
    pub(super) space_area: Score,
    pub(super) space_area_by_pieces: Score,
    /// Each side's knights, bishops and rooks multiplied by its own pawn
    /// count, side-relative, so a fit can bend a piece's worth with the
    /// pawns on the board; and its bishops multiplied by the friendly
    /// pawns on their square colour.
    pub(super) knight_pawns: Score,
    pub(super) bishop_pawns: Score,
    pub(super) rook_pawns: Score,
    pub(super) bishop_pawns_on_colour: Score,
    /// Moves onto squares an enemy pawn attacks, side-relative, for
    /// knights, bishops, rooks and queens in turn. The raw move counts
    /// above include these squares; this says how many of them there were.
    pub(super) unsafe_mobility: [Score; 4],
    /// Passers with a friendly rook behind them on the file and nothing
    /// between, side-relative. The path blocks by rank are weighted into
    /// the piece-indexed pair where they are produced.
    pub(super) rook_behind_passer: Score,
    /// Enemy pieces a friendly pawn would attack after a safe push, and the
    /// sides a colour may still castle to, both side-relative.
    pub(super) threat_by_pawn_push: Score,
    pub(super) castling_rights: Score,
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
    let base = weights::score(&features)
        + weights::profile_mobility_adjustment(&features)
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
    ((forcing.max(0) * Score::from(config.aggression()) + 50) / 100 / 4).min(12)
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
    let base = weights::score(&features)
        + weights::profile_mobility_adjustment(&features)
            .scaled(config.mobility_profile_intensity());
    let style = weights::attacking_style(&features)
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
        assert_eq!(weights::score(&plain), weights::score(&styled));
        assert_eq!(
            weights::profile_mobility_adjustment(&plain),
            weights::profile_mobility_adjustment(&styled),
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
        assert!(weights::attacking_style(&attacking).middle_game > 0);
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
            super::weights::score(&generic),
            super::weights::score(&EvalFeatures::default())
        );
        assert_eq!(
            super::weights::score(&pieces),
            super::weights::score(&EvalFeatures::default())
        );
        assert_ne!(
            super::weights::profile_mobility_adjustment(&pieces),
            super::weights::profile_mobility_adjustment(&EvalFeatures::default())
        );
        assert_eq!(
            super::weights::attacking_style(&pieces),
            super::weights::attacking_style(&EvalFeatures::default())
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
            + super::weights::profile_mobility_adjustment(&features);
        let explicit = super::ScorePair::new(4, 4) * features.knight_mobility
            + super::ScorePair::new(5, 5) * features.bishop_mobility
            + super::ScorePair::new(2, 4) * features.rook_mobility
            + super::ScorePair::new(1, 2) * features.queen_mobility;
        assert_eq!(profiled, explicit);
    }

    /// The curves are what the objective score reads for the four piece
    /// types with one, and pawns and kings have no curve.
    #[test]
    fn mobility_curves_are_read_per_piece_type() {
        for piece in [Piece::Pawn, Piece::King] {
            for count in 0..super::QUEEN_MOBILITY_ENTRIES {
                assert_eq!(
                    super::weights::mobility_curve(piece, count),
                    super::ScorePair::new(0, 0)
                );
            }
        }
        // A piece with every square it could have is worth more in the
        // ending than one with none, for each of the four curves.
        for (piece, entries) in [
            (Piece::Knight, super::KNIGHT_MOBILITY_ENTRIES),
            (Piece::Bishop, super::BISHOP_MOBILITY_ENTRIES),
            (Piece::Rook, super::ROOK_MOBILITY_ENTRIES),
            (Piece::Queen, super::QUEEN_MOBILITY_ENTRIES),
        ] {
            let trapped = super::weights::mobility_curve(piece, 0);
            let free = super::weights::mobility_curve(piece, entries - 1);
            assert!(free.end_game() > trapped.end_game(), "{piece:?}");
        }

        // In a real position the accumulated pair is what the score reads,
        // and the scalar counts still describe the same pieces.
        let position = Position::from_fen(
            "r1bqkb1r/pppp1ppp/2n2n2/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4",
        )
        .unwrap();
        let features = evaluate_with_trace(position.board()).features;
        let curved = features.knight_mobility
            + features.bishop_mobility
            + features.rook_mobility
            + features.queen_mobility;
        assert_eq!(
            features.mobility,
            curved + features.pawn_mobility + features.king_mobility
        );
        let without = EvalFeatures {
            mobility_curves: super::ScorePair::new(0, 0),
            ..features
        };
        assert_eq!(
            super::weights::score(&features),
            super::weights::score(&without) + features.mobility_curves
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
    }

    #[test]
    fn outposts_need_pawn_support_and_no_pawn_challenge() {
        let features =
            |fen: &str| evaluate_with_trace(Position::from_fen(fen).unwrap().board()).features;

        // A knight on d5 held by e4, with no black pawn on the c or e files
        // ahead of it, is the textbook case.
        let held = features("4k3/pp3ppp/8/3N4/4P3/8/PP3PPP/4K3 w - - 0 1");
        assert_eq!(held.knight_outposts, 1);
        assert_eq!(held.bishop_outposts, 0);
        // An enemy pawn that can still advance to challenge it removes it.
        assert_eq!(
            features("4k3/ppp2ppp/8/3N4/4P3/8/PP3PPP/4K3 w - - 0 1").knight_outposts,
            0
        );
        // So does the absence of the defending pawn.
        assert_eq!(
            features("4k3/pp3ppp/8/3N4/8/8/PP3PPP/4K3 w - - 0 1").knight_outposts,
            0
        );
        // The fourth rank is the nearest an outpost may be; the third is not one.
        assert_eq!(
            features("4k3/pp3ppp/8/8/8/3N4/PP2PPPP/4K3 w - - 0 1").knight_outposts,
            0
        );
        // Black's outposts count against, on Black's own fourth to sixth.
        let black = features("4k3/pp3ppp/8/4p3/3n4/8/PP3PPP/4K3 w - - 0 1");
        assert_eq!(black.knight_outposts, -1);
        assert_eq!(
            features("4k3/pp3ppp/8/4p3/3b4/8/PP3PPP/4K3 w - - 0 1").bishop_outposts,
            -1
        );
    }

    #[test]
    fn backward_and_connected_pawns_are_counted_by_rank() {
        let counts =
            |fen: &str| super::features::structure_counts(Position::from_fen(fen).unwrap().board());

        // A phalanx on the fourth counts both pawns at rank index two.
        let phalanx = counts("4k3/8/8/8/3PP3/8/8/4K3 w - - 0 1");
        assert_eq!(phalanx.connected_by_rank, [0, 0, 2, 0, 0, 0]);
        // A pawn defended from behind is connected; its defender is not.
        let chain = counts("4k3/8/8/8/3P4/4P3/8/4K3 w - - 0 1");
        assert_eq!(chain.connected_by_rank, [0, 0, 1, 0, 0, 0]);
        assert_eq!(chain.backward, 0);

        // e3 cannot advance past d5's control and no pawn can come to help
        // it: backward. The counts are side-relative, so Black's pawns are
        // given support that keeps them out of the count. A friendly pawn
        // already ahead on an adjacent file does not help; one behind does.
        assert_eq!(counts("4k3/8/2p5/3p4/8/4P3/8/4K3 w - - 0 1").backward, 1);
        assert_eq!(
            counts("4k3/1p6/2p5/3p4/3P4/4P3/8/4K3 w - - 0 1").backward,
            1
        );
        assert_eq!(counts("4k3/8/2p5/3p4/8/4P3/5P2/4K3 w - - 0 1").backward, 0);
        // An enemy pawn standing on the stop square blocks it just as well.
        assert_eq!(counts("4k3/8/2p5/3p4/4p3/4P3/8/4K3 w - - 0 1").backward, 1);
        // Black's backward pawn counts against, on Black's own terms.
        assert_eq!(counts("4k3/8/4p3/8/3P4/2P5/8/4K3 w - - 0 1").backward, -1);
        // A black phalanx on the fifth is on Black's fourth: rank index two.
        assert_eq!(
            counts("4k3/8/8/3pp3/8/8/8/4K3 w - - 0 1").connected_by_rank,
            [0, 0, -2, 0, 0, 0]
        );
        // The engine reads the same count.
        let features = evaluate_with_trace(
            Position::from_fen("4k3/8/2p5/3p4/8/4P3/8/4K3 w - - 0 1")
                .unwrap()
                .board(),
        )
        .features;
        assert_eq!(features.backward_pawns, 1);
    }

    #[test]
    fn passers_report_blockade_and_king_distance() {
        let counts =
            |fen: &str| super::features::structure_counts(Position::from_fen(fen).unwrap().board());
        let blocked = |fen: &str| {
            let summary = super::features::attack_summary(Position::from_fen(fen).unwrap().board());
            let [white, black] = summary.blocked_passers;
            std::array::from_fn::<Score, 6, _>(|rank| white[rank] - black[rank])
        };

        // A free passer on d4: its stop square d5 is four king moves from e1
        // and three from e8.
        let free = counts("4k3/8/8/8/3P4/8/8/4K3 w - - 0 1");
        assert_eq!(free.passed_by_rank, [0, 0, 1, 0, 0, 0]);
        assert_eq!(blocked("4k3/8/8/8/3P4/8/8/4K3 w - - 0 1"), [0; 6]);
        assert_eq!(free.passer_own_king_distance, [0, 0, 0, 0, 1, 0, 0, 0]);
        assert_eq!(free.passer_enemy_king_distance, [0, 0, 0, 1, 0, 0, 0, 0]);

        // Any piece on the square ahead is a blockade, the owner's own
        // included.
        assert_eq!(
            blocked("4k3/8/8/3n4/3P4/8/8/4K3 w - - 0 1"),
            [0, 0, 1, 0, 0, 0]
        );
        assert_eq!(
            blocked("4k3/8/8/3N4/3P4/8/8/4K3 w - - 0 1"),
            [0, 0, 1, 0, 0, 0]
        );
        // A pawn that is not passed is not counted whatever stands ahead.
        assert_eq!(blocked("4k3/8/2p5/3n4/3P4/8/8/4K3 w - - 0 1"), [0; 6]);
        // Black's blockaded passer counts against.
        assert_eq!(
            blocked("4k3/8/8/8/3p4/3N4/8/4K3 w - - 0 1"),
            [0, 0, 0, -1, 0, 0]
        );

        // Black's passer on d4 is on Black's fifth; its stop square d3 is five
        // from e8 and two from e1, counted with the opposite sign.
        let black = counts("4k3/8/8/8/3p4/8/8/4K3 w - - 0 1");
        assert_eq!(black.passed_by_rank, [0, 0, 0, -1, 0, 0]);
        assert_eq!(black.passer_own_king_distance, [0, 0, 0, 0, 0, -1, 0, 0]);
        assert_eq!(black.passer_enemy_king_distance, [0, 0, -1, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn king_danger_counts_attack_units_and_safe_checks() {
        let summary =
            |fen: &str| super::features::attack_summary(Position::from_fen(fen).unwrap().board());
        let bucket = |units: i32| super::features::king_danger_bucket(units);
        let safe_checks = |fen: &str| {
            let [white, black] = summary(fen).safe_checks;
            std::array::from_fn::<Score, 4, _>(|slot| white[slot] - black[slot])
        };

        // Nothing touches either king zone in the starting position.
        let units = |fen: &str| summary(fen).scans.map(|scan| scan.attack_units());
        let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        assert_eq!(units(start), [0, 0]);
        assert_eq!(summary(start).safe_checks, [[0; 4]; 2]);

        // A queen bearing on the king zone lands White above the first
        // bucket, and the mirrored position lands Black in the same one.
        let white = units("4k3/8/3Q4/8/8/8/8/4K3 w - - 0 1");
        let black = units("4k3/8/8/8/8/3q4/8/4K3 w - - 0 1");
        assert!(white[0] > 0 && bucket(white[0]) > 0);
        assert_eq!(white[1], 0);
        assert_eq!(black, [white[1], white[0]]);
        // More attackers land in a higher bucket, and the bucket saturates.
        let assault = units("4k3/8/3Q4/5N2/8/8/8/4K3 w - - 0 1");
        assert!(bucket(assault[0]) > bucket(white[0]));
        assert_eq!(bucket(1_000), super::KING_DANGER_BUCKETS - 1);

        // A rook on a1 checks safely from a8. A queen on d1 checks from a4,
        // h5 and e2 but not from d8, which only the king covers and nothing
        // of White's supports.
        assert_eq!(safe_checks("4k3/8/8/8/8/8/8/R3K3 w - - 0 1"), [0, 0, 1, 0]);
        assert_eq!(safe_checks("4k3/8/8/8/8/8/8/3QK3 w - - 0 1"), [0, 0, 0, 3]);
        // With a bishop on b6 covering d8 as well, that check becomes safe.
        assert_eq!(
            safe_checks("4k3/8/1B6/8/8/8/8/3QK3 w - - 0 1"),
            [0, 0, 0, 4]
        );
        // A knight on e4 checks from d6 and f6; a pawn on e7 covers both.
        assert_eq!(safe_checks("4k3/8/8/8/4N3/8/8/4K3 w - - 0 1"), [2, 0, 0, 0]);
        assert_eq!(safe_checks("4k3/4p3/8/8/4N3/8/8/4K3 w - - 0 1"), [0; 4]);
        // Black's checks count against.
        assert_eq!(safe_checks("4k3/8/8/8/8/8/r7/4K3 w - - 0 1"), [0, 0, -1, 0]);
    }

    #[test]
    fn shelter_is_graded_by_the_nearest_pawn_on_each_file() {
        let counts =
            |fen: &str| super::features::structure_counts(Position::from_fen(fen).unwrap().board());

        let unmoved = counts("4k3/8/8/8/8/8/4P3/4K3 w - - 0 1");
        assert_eq!(unmoved.shelter_king_file_by_distance, [1, 0, 0, 0, 0, 0]);
        assert_eq!(unmoved.shelter_adjacent_file_by_distance, [0; 6]);
        let advanced = counts("4k3/8/8/8/8/5P2/8/4K3 w - - 0 1");
        assert_eq!(advanced.shelter_king_file_by_distance, [0; 6]);
        assert_eq!(
            advanced.shelter_adjacent_file_by_distance,
            [0, 1, 0, 0, 0, 0]
        );
        // Only the nearest pawn on a file counts, and a pawn level with or
        // behind the king is not shelter.
        assert_eq!(
            counts("4k3/8/8/8/4P3/8/4P3/4K3 w - - 0 1").shelter_king_file_by_distance,
            [1, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            counts("4k3/8/8/8/8/4K3/3P4/8 w - - 0 1").shelter_adjacent_file_by_distance,
            [0; 6]
        );
        // Black's shelter counts against, measured from Black's side.
        assert_eq!(
            counts("4k3/3p4/8/8/8/8/8/4K3 w - - 0 1").shelter_adjacent_file_by_distance,
            [-1, 0, 0, 0, 0, 0]
        );
        // The shelter count is unchanged by the grading.
        assert_eq!(unmoved.shelter, 1);
    }

    #[test]
    fn the_storm_is_graded_by_the_nearest_enemy_pawn_on_each_file() {
        let counts =
            |fen: &str| super::features::structure_counts(Position::from_fen(fen).unwrap().board());

        // An enemy pawn two ranks ahead on the king's file with nothing in
        // its path, then one three ranks ahead on the file beside it.
        let open = counts("k7/8/8/8/8/4p3/8/4K3 w - - 0 1");
        assert_eq!(open.storm_king_file_by_distance, [0, 1, 0, 0, 0, 0]);
        assert_eq!(open.storm_adjacent_file_by_distance, [0; 6]);
        assert_eq!(open.blocked_storm_by_distance, [0; 6]);
        let flank = counts("k7/8/8/8/5p2/8/8/4K3 w - - 0 1");
        assert_eq!(flank.storm_king_file_by_distance, [0; 6]);
        assert_eq!(flank.storm_adjacent_file_by_distance, [0, 0, 1, 0, 0, 0]);
        assert_eq!(flank.blocked_storm_by_distance, [0; 6]);
        // A friendly pawn directly in its path makes it a blocked storm,
        // counted apart from both file blocks.
        let blocked = counts("k7/8/8/8/8/4p3/4P3/4K3 w - - 0 1");
        assert_eq!(blocked.storm_king_file_by_distance, [0; 6]);
        assert_eq!(blocked.storm_adjacent_file_by_distance, [0; 6]);
        assert_eq!(blocked.blocked_storm_by_distance, [0, 1, 0, 0, 0, 0]);
        // Only the nearest pawn on a file counts.
        assert_eq!(
            counts("k7/8/8/4p3/8/4p3/8/4K3 w - - 0 1").storm_king_file_by_distance,
            [0, 1, 0, 0, 0, 0]
        );
        // A pawn level with or behind the king has passed it.
        let passed = counts("k7/8/8/8/8/4K3/3p4/8 w - - 0 1");
        assert_eq!(passed.storm_king_file_by_distance, [0; 6]);
        assert_eq!(passed.storm_adjacent_file_by_distance, [0; 6]);
        assert_eq!(passed.blocked_storm_by_distance, [0; 6]);
        // Black's storm counts against, measured from Black's side.
        assert_eq!(
            counts("4k3/8/8/8/8/8/3P4/K7 w - - 0 1").storm_adjacent_file_by_distance,
            [0, 0, 0, 0, 0, -1]
        );
        assert_eq!(
            counts("4k3/8/4p3/4P3/8/8/8/K7 w - - 0 1").blocked_storm_by_distance,
            [0, 0, -1, 0, 0, 0]
        );
        // The shelter is unchanged by the storm.
        assert_eq!(blocked.shelter_king_file_by_distance, [1, 0, 0, 0, 0, 0]);
        assert_eq!(blocked.shelter, 1);
    }

    #[test]
    fn space_counts_safe_centre_squares_behind_the_pawns() {
        let counts =
            |fen: &str| super::features::structure_counts(Position::from_fen(fen).unwrap().board());

        // Twelve centre squares a side, four holding its own pawns.
        assert_eq!(
            counts("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").space_area,
            [8, 8]
        );
        // After 1.e4 the pawn still stands in White's zone, the vacated e2
        // and e3 behind it count twice, and its attacks on d5 and f5 cost
        // Black two squares.
        assert_eq!(
            counts("rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1").space_area,
            [10, 6]
        );
        // After 1.e4 d5 each pawn attacks one free square of the other's
        // zone, and each side has two squares behind its advanced pawn.
        assert_eq!(
            counts("rnbqkbnr/ppp1pppp/8/3p4/4P3/8/PPPP1PPP/RNBQKBNR w KQkq d6 0 2").space_area,
            [9, 9]
        );

        // The extraction carries the signed area and the area scaled by
        // the owner's pieces, so a side with no pieces earns nothing for
        // its room.
        let features = |fen: &str| {
            super::features::extract_with_style(Position::from_fen(fen).unwrap().board(), false)
        };
        let bare = features("4k3/8/8/8/8/8/2PPPP2/4K3 w - - 0 1");
        assert_eq!(bare.space_area, 8 - 12);
        assert_eq!(bare.space_area_by_pieces, 0);
        let knight = features("4k3/8/8/8/8/8/2PPPP2/4K1N1 w - - 0 1");
        assert_eq!(knight.space_area, 8 - 12);
        assert_eq!(knight.space_area_by_pieces, 8);
    }

    #[test]
    fn candidate_passers_islands_and_pawnless_flanks_are_counted() {
        let counts =
            |fen: &str| super::features::structure_counts(Position::from_fen(fen).unwrap().board());

        // e4 is held back by d5, but its file is clear and d3 can match the
        // sentry: a candidate on the fourth, rank index two. Without d3 it
        // is not, and a pawn that is already passed is not a candidate.
        let candidate = counts("4k3/8/8/3p4/4P3/3P4/8/4K3 w - - 0 1");
        assert_eq!(candidate.candidate_passer_by_rank, [0, 0, 1, 0, 0, 0]);
        assert_eq!(
            counts("4k3/8/8/3p4/4P3/8/8/4K3 w - - 0 1").candidate_passer_by_rank,
            [0; 6]
        );
        let passed = counts("4k3/8/8/8/4P3/8/8/4K3 w - - 0 1");
        assert_eq!(passed.candidate_passer_by_rank, [0; 6]);
        assert_eq!(passed.passed_by_rank, [0, 0, 1, 0, 0, 0]);
        // An enemy pawn on the file ahead rules a pawn out however many
        // helpers it has.
        assert_eq!(
            counts("4k3/8/4p3/8/4P3/3P1P2/8/4K3 w - - 0 1").candidate_passer_by_rank,
            [0; 6]
        );
        // Black's candidate on its own fourth counts against.
        assert_eq!(
            counts("4k3/8/3p4/4p3/3P4/8/8/4K3 w - - 0 1").candidate_passer_by_rank,
            [0, 0, -1, 0, 0, 0]
        );

        // Islands are runs of occupied files, side-relative.
        assert_eq!(
            counts("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").pawn_islands,
            0
        );
        assert_eq!(
            counts("4k3/8/8/8/8/8/P1P1P1P1/4K3 w - - 0 1").pawn_islands,
            4
        );
        assert_eq!(
            counts("4k3/pp3ppp/8/8/8/8/PPPPPPPP/4K3 w - - 0 1").pawn_islands,
            -1
        );

        // A flank is the four files nearest the king, and it is pawnless
        // only with no pawn of either colour on it.
        assert_eq!(counts("k7/8/8/8/8/8/8/7K w - - 0 1").king_pawnless_flank, 0);
        assert_eq!(
            counts("k7/8/8/8/8/8/4P3/7K w - - 0 1").king_pawnless_flank,
            -1
        );
        assert_eq!(
            counts("k7/p7/8/8/8/8/8/7K w - - 0 1").king_pawnless_flank,
            1
        );
        assert_eq!(
            counts("3k4/8/8/8/8/8/1P6/K7 w - - 0 1").king_pawnless_flank,
            -1
        );
        assert_eq!(
            counts("4k3/8/8/8/8/8/1P4P1/K7 w - - 0 1").king_pawnless_flank,
            -1
        );
        assert_eq!(
            counts("5k2/8/8/8/8/8/1P6/K7 w - - 0 1").king_pawnless_flank,
            -1
        );
        assert_eq!(
            counts("2k5/8/8/8/8/8/3P4/7K w - - 0 1").king_pawnless_flank,
            1
        );
    }

    #[test]
    fn piece_values_scale_with_the_pawn_count() {
        let features = |fen: &str| {
            super::features::extract_with_style(Position::from_fen(fen).unwrap().board(), false)
        };

        // One knight and three pawns; one rook and two.
        let knight = features("4k3/8/8/8/8/8/PPP5/4K1N1 w - - 0 1");
        assert_eq!(knight.knight_pawns, 3);
        assert_eq!(knight.bishop_pawns, 0);
        assert_eq!(knight.rook_pawns, 0);
        let rook = features("4k3/8/8/8/8/8/PP6/R3K3 w - - 0 1");
        assert_eq!(rook.rook_pawns, 2);
        // A bishop counts the friendly pawns on its own colour: none for a
        // dark-squared bishop behind four light-square pawns, all four for
        // the light-squared one.
        let dark = features("4k3/8/8/8/8/8/P1P1P1P1/2B1K3 w - - 0 1");
        assert_eq!(dark.bishop_pawns, 4);
        assert_eq!(dark.bishop_pawns_on_colour, 0);
        let light = features("4k3/8/8/8/8/8/P1P1P1P1/1B2K3 w - - 0 1");
        assert_eq!(light.bishop_pawns, 4);
        assert_eq!(light.bishop_pawns_on_colour, 4);
        // Black's count against, with its own pawns.
        let black = features("1b2k3/p1p1p1p1/8/8/8/8/8/4K3 w - - 0 1");
        assert_eq!(black.bishop_pawns, -4);
        assert_eq!(black.bishop_pawns_on_colour, -4);
        assert_eq!(black.knight_pawns, 0);
    }

    #[test]
    fn mobility_onto_pawn_attacked_squares_is_counted_by_piece() {
        let features = |fen: &str| {
            super::features::extract_with_style(Position::from_fen(fen).unwrap().board(), false)
        };

        // A knight on d4 has eight moves; the pawn on e6 guards f5.
        let knight = features("4k3/8/4p3/8/3N4/8/8/4K3 w - - 0 1");
        assert_eq!(knight.knight_mobility, 8);
        assert_eq!(knight.unsafe_mobility, [1, 0, 0, 0]);
        // A bishop on b1 reaches e4, which the pawn on d5 guards.
        assert_eq!(
            features("4k3/8/8/3p4/8/8/8/1B2K3 w - - 0 1").unsafe_mobility,
            [0, 1, 0, 0]
        );
        // A rook on a1 reaches a2, which the pawn on b3 guards.
        assert_eq!(
            features("4k3/8/8/8/8/1p6/8/R3K3 w - - 0 1").unsafe_mobility,
            [0, 0, 1, 0]
        );
        // A queen on d1 reaches d2, which the pawn on e3 guards.
        assert_eq!(
            features("4k3/8/8/8/8/4p3/8/3QK3 w - - 0 1").unsafe_mobility,
            [0, 0, 0, 1]
        );
        // A guarded square a friendly piece occupies is not a move at all.
        assert_eq!(
            features("4k3/8/4p3/5P2/3N4/8/8/4K3 w - - 0 1").unsafe_mobility,
            [0; 4]
        );
        // Black's count against.
        assert_eq!(
            features("4k3/8/8/3n4/8/4P3/8/4K3 w - - 0 1").unsafe_mobility,
            [-1, 0, 0, 0]
        );
    }

    #[test]
    fn passers_are_scored_by_the_state_of_their_path() {
        let summary = |fen: &str| {
            super::features::attack_summary_with_style(
                Position::from_fen(fen).unwrap().board(),
                false,
            )
        };
        let features = |fen: &str| {
            super::features::extract_with_style(Position::from_fen(fen).unwrap().board(), false)
        };

        // A passer on the fourth with nothing ahead: its path is safe and
        // free, rank index two.
        let open = summary("k7/8/8/8/4P3/8/8/K7 w - - 0 1");
        assert_eq!(open.passer_safe_path[0], [0, 0, 1, 0, 0, 0]);
        assert_eq!(open.passer_free_path[0], [0, 0, 1, 0, 0, 0]);
        assert_eq!(open.rook_behind_passer, [0, 0]);
        // A rook on the promotion square stands on the path and attacks
        // the rest of it; one across the path attacks a square of it.
        let blocked = summary("k3r3/8/8/8/4P3/8/8/K7 w - - 0 1");
        assert_eq!(blocked.passer_safe_path[0], [0; 6]);
        assert_eq!(blocked.passer_free_path[0], [0; 6]);
        let crossed = summary("k7/8/8/7r/4P3/8/8/K7 w - - 0 1");
        assert_eq!(crossed.passer_safe_path[0], [0; 6]);
        assert_eq!(crossed.passer_free_path[0], [0, 0, 1, 0, 0, 0]);
        // The enemy king's reach counts, which the attack map leaves out.
        assert_eq!(
            summary("4k3/8/8/8/4P3/8/8/K7 w - - 0 1").passer_safe_path[0],
            [0; 6]
        );
        // A rook behind the passer counts with the file clear between them,
        // not with a piece in the way, and not from in front.
        assert_eq!(
            summary("k7/8/8/8/4P3/8/8/K3R3 w - - 0 1").rook_behind_passer,
            [1, 0]
        );
        assert_eq!(
            summary("k7/8/8/8/4P3/4N3/8/K3R3 w - - 0 1").rook_behind_passer,
            [0, 0]
        );
        assert_eq!(
            summary("k7/4R3/8/8/4P3/8/8/K7 w - - 0 1").rook_behind_passer,
            [0, 0]
        );
        // Black's passer on its own sixth, rank index four, with its rook
        // behind it, counts against.
        let black = summary("k3r3/8/8/8/8/4p3/8/K7 w - - 0 1");
        assert_eq!(black.passer_safe_path[1], [0, 0, 0, 0, 1, 0]);
        assert_eq!(black.passer_free_path[1], [0, 0, 0, 0, 1, 0]);
        assert_eq!(black.rook_behind_passer, [0, 1]);
        assert_eq!(
            features("k3r3/8/8/8/8/4p3/8/K7 w - - 0 1").rook_behind_passer,
            -1
        );
    }

    #[test]
    fn pawn_push_threats_and_castling_rights_are_counted() {
        let threats = |fen: &str| {
            super::features::attack_summary_with_style(
                Position::from_fen(fen).unwrap().board(),
                false,
            )
            .pawn_push_threats
        };
        let features = |fen: &str| {
            super::features::extract_with_style(Position::from_fen(fen).unwrap().board(), false)
        };

        // e2-e3 would attack the knight on d4.
        assert_eq!(threats("4k3/8/8/8/3n4/8/4P3/4K3 w - - 0 1"), [1, 0]);
        // Not onto a square a rook attacks and nothing defends; onto it
        // once the king defends it.
        assert_eq!(threats("4k3/4r3/8/8/3n4/8/4P3/4K3 w - - 0 1"), [0, 0]);
        assert_eq!(threats("4k3/4r3/8/8/3n4/8/3KP3/8 w - - 0 1"), [1, 0]);
        // A double push from the second rank counts, through an empty
        // square.
        assert_eq!(threats("4k3/8/8/3n4/8/8/4P3/4K3 w - - 0 1"), [1, 0]);
        assert_eq!(threats("4k3/8/8/3n4/8/4B3/4P3/4K3 w - - 0 1"), [0, 0]);
        // Black's double push e7-e5 would attack the knight on d4.
        assert_eq!(threats("4k3/4p3/8/8/3N4/8/8/4K3 w - - 0 1"), [0, 1]);
        assert_eq!(
            features("4k3/4p3/8/8/3N4/8/8/4K3 w - - 0 1").threat_by_pawn_push,
            -1
        );

        // Castling rights are counted per side.
        assert_eq!(
            features("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").castling_rights,
            0
        );
        assert_eq!(
            features("r3k2r/8/8/8/8/8/8/R3K2R w KQ - 0 1").castling_rights,
            2
        );
        assert_eq!(
            features("r3k2r/8/8/8/8/8/8/R3K2R w k - 0 1").castling_rights,
            -1
        );
    }

    #[test]
    fn tropism_buckets_each_piece_by_its_distance_to_the_enemy_king() {
        let counts =
            |fen: &str| super::features::tropism_counts(Position::from_fen(fen).unwrap().board());

        // A knight one square from the king, a bishop two, a rook three,
        // and a queen seven, which shares the last bucket with four.
        let mut expected = [0; 16];
        expected[0] = 1;
        expected[4 + 1] = 1;
        expected[8 + 2] = 1;
        expected[12 + 3] = 1;
        assert_eq!(counts("4k3/3N4/2B5/1R6/8/8/8/Q3K3 w - - 0 1"), expected);
        assert_eq!(
            counts("4k3/8/8/8/3Q4/8/8/4K3 w - - 0 1")[12 + 3],
            1,
            "distance four is the last bucket",
        );
        // Black's pieces count against, measured to White's king.
        let mut against = [0; 16];
        against[0] = -1;
        against[4 + 1] = -1;
        against[8 + 2] = -1;
        against[12 + 3] = -1;
        assert_eq!(counts("q3k3/8/8/8/1r6/2b5/3n4/4K3 w - - 0 1"), against);
    }

    #[test]
    fn objective_threats_count_attacked_pieces_by_kind() {
        let features =
            |fen: &str| evaluate_with_trace(Position::from_fen(fen).unwrap().board()).features;

        // A pawn on e4 attacks an undefended knight on d5: a minor attacked
        // by a pawn, and a hanging piece.
        let forked = features("4k3/8/8/3n4/4P3/8/8/4K3 w - - 0 1");
        assert_eq!(forked.threat_minor_by_pawn, 1);
        assert_eq!(forked.threat_hanging, 1);
        assert_eq!(forked.threat_by_lower_value, 0);
        // Defended by c6, it is still attacked by a pawn but no longer hangs.
        let defended = features("4k3/8/2p5/3n4/4P3/8/8/4K3 w - - 0 1");
        assert_eq!(defended.threat_minor_by_pawn, 1);
        assert_eq!(defended.threat_hanging, 0);
        // A knight on c3 attacks a rook on d5: worth less than its target.
        let rook = features("4k3/8/8/3r4/8/2N5/8/4K3 w - - 0 1");
        assert_eq!(rook.threat_by_lower_value, 1);
        assert_eq!(rook.threat_hanging, 1);
        // A rook on d1 attacks a queen on d5; the queen attacks the rook back
        // but is worth more, and the rook is defended by its king.
        let queen = features("4k3/8/8/3q4/8/8/8/3RK3 w - - 0 1");
        assert_eq!(queen.threat_by_lower_value, 1);
        assert_eq!(queen.threat_hanging, 1);
        // Black's threats count against.
        assert_eq!(
            features("4k3/8/8/8/3p4/4N3/8/4K3 w - - 0 1").threat_minor_by_pawn,
            -1
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
            weights::attacking_style(&coordinated_trace.features).middle_game
                > weights::attacking_style(&lone_trace.features).middle_game
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
            weights::attacking_style(&pressure),
            weights::attacking_style(&deficit)
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
