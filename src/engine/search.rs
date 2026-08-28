mod algorithm;
mod control;
mod see;
mod time;
mod transposition;

use std::time::Duration;

pub use control::SearchControl;
pub(crate) use time::TimeBudget;
pub(super) use time::{DEFAULT_MOVE_OVERHEAD_MS, MAX_MOVE_OVERHEAD_MS, MIN_MOVE_OVERHEAD_MS};
pub(super) use transposition::{DEFAULT_HASH_MIB, MAX_HASH_MIB, MIN_HASH_MIB, TranspositionTable};

/// Smallest supported number of search threads.
pub(super) const MIN_THREADS: usize = 1;
/// Number of search threads used unless configured otherwise.
///
/// One thread keeps a search deterministic, which every fixed-node fixture and
/// measurement gate depends on, so parallel search is opt-in.
pub(super) const DEFAULT_THREADS: usize = 1;
/// Largest supported number of search threads.
///
/// Each searcher keeps its own move ordering, per-ply tables, and move-picker
/// storage, so the ceiling bounds memory as well as oversubscription.
pub(super) const MAX_THREADS: usize = 128;

use super::Position;
use super::evaluation::{EvaluationConfig, MATE_SCORE, MATE_THRESHOLD, Score};

/// Limits supplied to a search operation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchLimits {
    /// Restricts the root search to these UCI moves when non-empty.
    pub search_moves: Vec<String>,
    /// Whether this is a ponder search.
    pub ponder: bool,
    /// White's remaining time.
    pub white_time: Option<Duration>,
    /// Black's remaining time.
    pub black_time: Option<Duration>,
    /// White's increment per move.
    pub white_increment: Option<Duration>,
    /// Black's increment per move.
    pub black_increment: Option<Duration>,
    /// Estimated moves remaining until the next time control.
    pub moves_to_go: Option<u32>,
    /// Maximum search depth in plies.
    pub depth: Option<u32>,
    /// Maximum number of searched nodes.
    pub nodes: Option<u64>,
    /// Requested mate-search depth in moves.
    pub mate: Option<u32>,
    /// Exact time allocated to this move.
    pub move_time: Option<Duration>,
    /// Overrides verified null-move pruning for controlled comparisons.
    pub null_move: Option<bool>,
    /// Whether to search until explicitly stopped.
    pub infinite: bool,
}

/// A score reported for a completed search iteration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchScore {
    /// A static score in centipawns from the root side's perspective.
    Centipawns(i32),
    /// A forced mate in the signed number of moves.
    Mate(i32),
}

impl SearchScore {
    fn from_internal(score: Score) -> Self {
        if score.abs() >= MATE_THRESHOLD {
            let plies = MATE_SCORE - score.abs();
            let moves = (plies + 1) / 2;
            Self::Mate(if score.is_negative() { -moves } else { moves })
        } else {
            Self::Centipawns(score)
        }
    }
}

/// Progress from one fully completed iterative-deepening pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchInfo {
    depth: u32,
    score: SearchScore,
    nodes: u64,
    elapsed: Duration,
    pv: Vec<String>,
}

impl SearchInfo {
    fn new(depth: u32, score: SearchScore, nodes: u64, elapsed: Duration, pv: Vec<String>) -> Self {
        Self {
            depth,
            score,
            nodes,
            elapsed,
            pv,
        }
    }

    /// Returns the completed depth in plies.
    #[must_use]
    pub fn depth(&self) -> u32 {
        self.depth
    }

    /// Returns the root-relative score.
    #[must_use]
    pub fn score(&self) -> SearchScore {
        self.score
    }

    /// Returns the cumulative searched node count.
    #[must_use]
    pub fn nodes(&self) -> u64 {
        self.nodes
    }

    /// Returns the elapsed search time.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// Returns the measured nodes per second.
    #[must_use]
    pub fn nodes_per_second(&self) -> u64 {
        let nanos = self.elapsed.as_nanos().max(1);
        (u128::from(self.nodes)
            .saturating_mul(1_000_000_000)
            .checked_div(nanos)
            .unwrap_or_default()
            .min(u128::from(u64::MAX))) as u64
    }

    /// Returns the principal variation in standard UCI notation.
    #[must_use]
    pub fn pv(&self) -> &[String] {
        &self.pv
    }
}

/// Verified null-move and selective-pruning work performed by a search.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchTelemetry {
    pub(super) null_move_attempts: u64,
    pub(super) null_move_fail_highs: u64,
    pub(super) null_move_verifications: u64,
    pub(super) null_move_cutoffs: u64,
    pub(super) null_probe_nodes: u64,
    pub(super) null_verification_nodes: u64,
    pub(super) static_pruning_attempts: u64,
    pub(super) static_evaluation_hits: u64,
    pub(super) reverse_futility_cutoffs: u64,
    pub(super) futility_pruned_moves: u64,
    pub(super) late_move_pruned_moves: u64,
    pub(super) aspiration_attempts: u64,
    pub(super) aspiration_fail_lows: u64,
    pub(super) aspiration_fail_highs: u64,
    pub(super) aspiration_research_nodes: u64,
    pub(super) legal_move_probes: u64,
    pub(super) tt_probes: u64,
    pub(super) tt_hits: u64,
    pub(super) tt_hash_moves: u64,
    pub(super) tt_cutoffs: u64,
    pub(super) quiescence_nodes: u64,
    pub(super) quiescence_pruned_captures: u64,
    pub(super) horizon_quiescence_pruned_captures: u64,
    pub(super) capture_cutoffs: u64,
    pub(super) capture_cutoff_index_sum: u64,
    pub(super) capture_history_updates: u64,
    pub(super) capture_history_first_move_cutoffs: u64,
    lmr_attempts: u64,
    lmr_reductions: u64,
    lmr_shallow_reductions: u64,
    lmr_researches: u64,
    lmr_shallow_researches: u64,
    lmr_research_fail_highs: u64,
    objective_root_nodes: u64,
    personality_root_nodes: u64,
    personality_verifications: u64,
}

impl SearchTelemetry {
    /// Returns the combined work of two searchers in one search.
    ///
    /// Every counter records work performed, so combining searchers is a sum.
    /// That keeps the reported relationships between counters true of the search
    /// as a whole, the same way they are true of each searcher individually.
    pub(super) fn merged_with(self, other: Self) -> Self {
        Self {
            null_move_attempts: self.null_move_attempts + other.null_move_attempts,
            null_move_fail_highs: self.null_move_fail_highs + other.null_move_fail_highs,
            null_move_verifications: self.null_move_verifications + other.null_move_verifications,
            null_move_cutoffs: self.null_move_cutoffs + other.null_move_cutoffs,
            null_probe_nodes: self.null_probe_nodes + other.null_probe_nodes,
            null_verification_nodes: self.null_verification_nodes + other.null_verification_nodes,
            static_pruning_attempts: self.static_pruning_attempts + other.static_pruning_attempts,
            static_evaluation_hits: self.static_evaluation_hits + other.static_evaluation_hits,
            reverse_futility_cutoffs: self.reverse_futility_cutoffs
                + other.reverse_futility_cutoffs,
            futility_pruned_moves: self.futility_pruned_moves + other.futility_pruned_moves,
            late_move_pruned_moves: self.late_move_pruned_moves + other.late_move_pruned_moves,
            aspiration_attempts: self.aspiration_attempts + other.aspiration_attempts,
            aspiration_fail_lows: self.aspiration_fail_lows + other.aspiration_fail_lows,
            aspiration_fail_highs: self.aspiration_fail_highs + other.aspiration_fail_highs,
            aspiration_research_nodes: self.aspiration_research_nodes
                + other.aspiration_research_nodes,
            legal_move_probes: self.legal_move_probes + other.legal_move_probes,
            tt_probes: self.tt_probes + other.tt_probes,
            tt_hits: self.tt_hits + other.tt_hits,
            tt_hash_moves: self.tt_hash_moves + other.tt_hash_moves,
            tt_cutoffs: self.tt_cutoffs + other.tt_cutoffs,
            quiescence_nodes: self.quiescence_nodes + other.quiescence_nodes,
            quiescence_pruned_captures: self.quiescence_pruned_captures
                + other.quiescence_pruned_captures,
            horizon_quiescence_pruned_captures: self.horizon_quiescence_pruned_captures
                + other.horizon_quiescence_pruned_captures,
            capture_cutoffs: self.capture_cutoffs + other.capture_cutoffs,
            capture_cutoff_index_sum: self.capture_cutoff_index_sum
                + other.capture_cutoff_index_sum,
            capture_history_updates: self.capture_history_updates + other.capture_history_updates,
            capture_history_first_move_cutoffs: self.capture_history_first_move_cutoffs
                + other.capture_history_first_move_cutoffs,
            lmr_attempts: self.lmr_attempts + other.lmr_attempts,
            lmr_reductions: self.lmr_reductions + other.lmr_reductions,
            lmr_shallow_reductions: self.lmr_shallow_reductions + other.lmr_shallow_reductions,
            lmr_researches: self.lmr_researches + other.lmr_researches,
            lmr_shallow_researches: self.lmr_shallow_researches + other.lmr_shallow_researches,
            lmr_research_fail_highs: self.lmr_research_fail_highs + other.lmr_research_fail_highs,
            objective_root_nodes: self.objective_root_nodes + other.objective_root_nodes,
            personality_root_nodes: self.personality_root_nodes + other.personality_root_nodes,
            personality_verifications: self.personality_verifications
                + other.personality_verifications,
        }
    }
}

impl SearchTelemetry {
    #[must_use]
    pub const fn null_move_attempts(self) -> u64 {
        self.null_move_attempts
    }

    #[must_use]
    pub const fn null_move_fail_highs(self) -> u64 {
        self.null_move_fail_highs
    }

    #[must_use]
    pub const fn null_move_verifications(self) -> u64 {
        self.null_move_verifications
    }

    #[must_use]
    pub const fn null_move_cutoffs(self) -> u64 {
        self.null_move_cutoffs
    }

    #[must_use]
    pub const fn null_probe_nodes(self) -> u64 {
        self.null_probe_nodes
    }

    #[must_use]
    pub const fn null_verification_nodes(self) -> u64 {
        self.null_verification_nodes
    }
    /// Returns the number of nodes considered for static pruning.
    #[must_use]
    pub const fn static_pruning_attempts(self) -> u64 {
        self.static_pruning_attempts
    }

    /// Returns the number of static evaluations recovered from the table.
    #[must_use]
    pub const fn static_evaluation_hits(self) -> u64 {
        self.static_evaluation_hits
    }

    /// Returns the number of reverse-futility node cutoffs.
    #[must_use]
    pub const fn reverse_futility_cutoffs(self) -> u64 {
        self.reverse_futility_cutoffs
    }

    /// Returns the number of late quiet moves skipped by futility pruning.
    #[must_use]
    pub const fn futility_pruned_moves(self) -> u64 {
        self.futility_pruned_moves
    }

    /// Returns the number of quiet moves skipped by move-count pruning.
    #[must_use]
    pub const fn late_move_pruned_moves(self) -> u64 {
        self.late_move_pruned_moves
    }

    /// Returns the number of moves considered for late-move reduction.
    #[must_use]
    pub const fn lmr_attempts(self) -> u64 {
        self.lmr_attempts
    }

    /// Returns the number of moves searched at reduced depth.
    #[must_use]
    pub const fn lmr_reductions(self) -> u64 {
        self.lmr_reductions
    }

    /// Returns the number of reduced moves searched again at full depth.
    #[must_use]
    pub const fn lmr_researches(self) -> u64 {
        self.lmr_researches
    }

    /// Returns the number of full-depth LMR re-searches that failed high.
    #[must_use]
    pub const fn lmr_research_fail_highs(self) -> u64 {
        self.lmr_research_fail_highs
    }

    /// Returns the nodes spent establishing conventional root results.
    #[must_use]
    pub const fn objective_root_nodes(self) -> u64 {
        self.objective_root_nodes
    }

    /// Returns the nodes spent probing and verifying styled root alternatives.
    #[must_use]
    pub const fn personality_root_nodes(self) -> u64 {
        self.personality_root_nodes
    }

    /// Returns the number of styled root candidates selected for verification.
    #[must_use]
    pub const fn personality_verifications(self) -> u64 {
        self.personality_verifications
    }

    /// Returns the number of finite-window root searches.
    #[must_use]
    pub const fn aspiration_attempts(self) -> u64 {
        self.aspiration_attempts
    }

    /// Returns the number of root searches that failed below alpha.
    #[must_use]
    pub const fn aspiration_fail_lows(self) -> u64 {
        self.aspiration_fail_lows
    }

    /// Returns the number of root searches that failed at or above beta.
    #[must_use]
    pub const fn aspiration_fail_highs(self) -> u64 {
        self.aspiration_fail_highs
    }

    /// Returns the nodes spent repeating a root search at the same depth.
    #[must_use]
    pub const fn aspiration_research_nodes(self) -> u64 {
        self.aspiration_research_nodes
    }

    /// Returns the number of full legal-move existence probes.
    #[must_use]
    pub const fn legal_move_probes(self) -> u64 {
        self.legal_move_probes
    }

    /// Returns the number of transposition-table probes made by search.
    #[must_use]
    pub const fn tt_probes(self) -> u64 {
        self.tt_probes
    }

    /// Returns the number of search probes that matched a table entry.
    #[must_use]
    pub const fn tt_hits(self) -> u64 {
        self.tt_hits
    }

    /// Returns the number of table hits that supplied a hash move.
    #[must_use]
    pub const fn tt_hash_moves(self) -> u64 {
        self.tt_hash_moves
    }

    /// Returns the number of table bounds that cut off a node.
    #[must_use]
    pub const fn tt_cutoffs(self) -> u64 {
        self.tt_cutoffs
    }

    /// Returns the number of nodes entered by quiescence search.
    #[must_use]
    pub const fn quiescence_nodes(self) -> u64 {
        self.quiescence_nodes
    }

    /// Returns the number of beta cutoffs caused by captures.
    #[must_use]
    pub const fn capture_cutoffs(self) -> u64 {
        self.capture_cutoffs
    }

    /// Returns the sum of zero-based move indexes for capture cutoffs.
    #[must_use]
    pub const fn capture_cutoff_index_sum(self) -> u64 {
        self.capture_cutoff_index_sum
    }

    /// Returns the number of learned capture-history cutoff updates.
    #[must_use]
    pub const fn capture_history_updates(self) -> u64 {
        self.capture_history_updates
    }

    /// Returns the number of eligible capture cutoffs before a failed capture.
    #[must_use]
    pub const fn capture_history_first_move_cutoffs(self) -> u64 {
        self.capture_history_first_move_cutoffs
    }

    /// Returns the number of captures discarded by quiescence pruning.
    #[must_use]
    pub const fn quiescence_pruned_captures(self) -> u64 {
        self.quiescence_pruned_captures
    }

    /// Returns the number of captures pruned on the first quiescence ply.
    #[must_use]
    pub const fn horizon_quiescence_pruned_captures(self) -> u64 {
        self.horizon_quiescence_pruned_captures
    }

    /// Returns the number of moves reduced at the shallow LMR boundary.
    #[must_use]
    pub const fn lmr_shallow_reductions(self) -> u64 {
        self.lmr_shallow_reductions
    }

    /// Returns the number of shallow reduced moves searched at full depth.
    #[must_use]
    pub const fn lmr_shallow_researches(self) -> u64 {
        self.lmr_shallow_researches
    }
}

/// The principal result produced by a search.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchResult {
    best_move: Option<String>,
    ponder: Option<String>,
    info: Option<SearchInfo>,
    telemetry: SearchTelemetry,
}

impl SearchResult {
    fn from_parts(
        best_move: Option<String>,
        info: Option<SearchInfo>,
        telemetry: SearchTelemetry,
    ) -> Self {
        let ponder = info
            .as_ref()
            .and_then(|search_info| search_info.pv().get(1).cloned());
        Self {
            best_move,
            ponder,
            info,
            telemetry,
        }
    }

    /// Returns the selected move in standard UCI notation.
    #[must_use]
    pub fn best_move(&self) -> Option<&str> {
        self.best_move.as_deref()
    }

    /// Returns the expected reply in standard UCI notation, when available.
    #[must_use]
    pub fn ponder(&self) -> Option<&str> {
        self.ponder.as_deref()
    }

    /// Returns the final completed iteration, when one finished.
    #[must_use]
    pub fn info(&self) -> Option<&SearchInfo> {
        self.info.as_ref()
    }

    #[must_use]
    pub const fn telemetry(&self) -> SearchTelemetry {
        self.telemetry
    }
}

/// Settings a search reads that are configured rather than derived per move.
#[derive(Clone, Copy, Debug)]
pub(super) struct SearchSettings {
    pub(super) evaluation: EvaluationConfig,
    pub(super) move_overhead: Duration,
    pub(super) threads: usize,
}

impl SearchSettings {
    #[cfg(test)]
    fn for_test(evaluation: EvaluationConfig) -> Self {
        Self {
            evaluation,
            move_overhead: Duration::from_millis(DEFAULT_MOVE_OVERHEAD_MS),
            threads: DEFAULT_THREADS,
        }
    }
}

#[cfg(test)]
pub(super) fn search(position: &Position, limits: &SearchLimits) -> SearchResult {
    let control = SearchControl::new();
    search_with_reporter(position, limits, &control, |_| {})
}

#[cfg(test)]
pub(super) fn search_with_reporter<F>(
    position: &Position,
    limits: &SearchLimits,
    control: &SearchControl,
    report: F,
) -> SearchResult
where
    F: FnMut(SearchInfo),
{
    let table = TranspositionTable::new(MIN_HASH_MIB)
        .expect("the minimum transposition table must be allocatable");
    search_with_table(
        position,
        limits,
        control,
        SearchSettings::for_test(EvaluationConfig::default()),
        &table,
        report,
    )
}

pub(super) fn search_with_table<F>(
    position: &Position,
    limits: &SearchLimits,
    control: &SearchControl,
    settings: SearchSettings,
    table: &TranspositionTable,
    report: F,
) -> SearchResult
where
    F: FnMut(SearchInfo),
{
    algorithm::run(position, limits, control, settings, table, report)
}

pub(super) fn time_budget(
    position: &Position,
    limits: &SearchLimits,
    move_overhead: Duration,
) -> Option<TimeBudget> {
    time::allocate_time(position.board().side_to_move(), limits, move_overhead)
}
pub(super) fn ponder_time_budget(
    position: &Position,
    limits: &SearchLimits,
    move_overhead: Duration,
) -> Option<TimeBudget> {
    time::allocate_time_after_ponder(position.board().side_to_move(), limits, move_overhead)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        EvaluationConfig, SearchControl, SearchInfo, SearchLimits, SearchScore, SearchSettings,
        TranspositionTable, search, search_with_reporter, search_with_table,
    };
    use crate::engine::Position;

    #[test]
    fn iterative_search_is_deterministic() {
        let position = Position::default();

        let first = search(&position, &SearchLimits::default());
        let second = search(&position, &SearchLimits::default());

        assert_eq!(first.best_move(), second.best_move());
        assert!(
            position
                .legal_moves()
                .contains(&first.best_move().unwrap().to_owned())
        );
    }

    #[test]
    fn verified_null_move_reports_attempts_and_cutoffs() {
        let position = Position::default();
        let control = SearchControl::new();
        let disabled_table = TranspositionTable::new(1).unwrap();
        let enabled_table = TranspositionTable::new(1).unwrap();
        let disabled = search_with_table(
            &position,
            &SearchLimits {
                nodes: Some(20_000),
                null_move: Some(false),
                ..SearchLimits::default()
            },
            &control,
            SearchSettings::for_test(EvaluationConfig::new(0)),
            &disabled_table,
            |_| {},
        );
        let enabled = search_with_table(
            &position,
            &SearchLimits {
                nodes: Some(20_000),
                null_move: Some(true),
                ..SearchLimits::default()
            },
            &control,
            SearchSettings::for_test(EvaluationConfig::new(0)),
            &enabled_table,
            |_| {},
        );

        let disabled = disabled.telemetry();
        let enabled = enabled.telemetry();
        assert_eq!(disabled.null_move_attempts(), 0);
        assert_eq!(disabled.null_move_cutoffs(), 0);
        assert!(enabled.null_move_attempts() > 0);
        assert_eq!(
            enabled.null_move_fail_highs(),
            enabled.null_move_verifications()
        );
        assert!(enabled.null_move_cutoffs() > 0);
        assert!(enabled.null_move_verifications() >= enabled.null_move_cutoffs());
        assert!(enabled.null_probe_nodes() >= enabled.null_move_attempts());
        assert!(enabled.null_verification_nodes() >= enabled.null_move_verifications());
    }
    #[test]
    fn a_reached_soft_deadline_stops_after_a_stable_iteration() {
        let position = Position::default();
        let control = SearchControl::new();
        control.set_time_budget_from_now(Duration::ZERO, Duration::from_secs(1));

        let result = search_with_reporter(
            &position,
            &SearchLimits {
                depth: Some(4),
                ..SearchLimits::default()
            },
            &control,
            |_| {},
        );

        assert_eq!(result.info().unwrap().depth(), 1);
        assert!(!control.hard_deadline_reached());
    }
    #[test]
    fn root_style_preference_does_not_change_draw_scores() {
        let position = Position::from_fen("6k1/5ppp/8/7Q/2B5/8/5PPP/6K1 w - - 99 1").unwrap();
        let limits = SearchLimits {
            search_moves: vec!["h5g5".to_owned()],
            depth: Some(1),
            ..SearchLimits::default()
        };

        let result = search(&position, &limits);

        assert_eq!(result.info().unwrap().score(), SearchScore::Centipawns(0));
    }

    #[test]
    fn a_warm_transposition_table_reduces_nodes_without_changing_the_result() {
        let position =
            Position::from_fen("r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3")
                .unwrap();
        let limits = SearchLimits {
            depth: Some(5),
            ..SearchLimits::default()
        };
        let control = SearchControl::new();
        let table = TranspositionTable::new(1).unwrap();

        let cold = search_with_table(
            &position,
            &limits,
            &control,
            SearchSettings::for_test(EvaluationConfig::default()),
            &table,
            |_| {},
        );
        let warm = search_with_table(
            &position,
            &limits,
            &control,
            SearchSettings::for_test(EvaluationConfig::default()),
            &table,
            |_| {},
        );

        assert_eq!(warm.best_move(), cold.best_move());
        assert_eq!(warm.info().unwrap().score(), cold.info().unwrap().score());
        assert!(warm.info().unwrap().nodes() < cold.info().unwrap().nodes());
    }
    #[test]
    fn aggression_changes_clear_policy_specific_hash_entries() {
        let position =
            Position::from_fen("rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2")
                .unwrap();
        let limits = SearchLimits {
            depth: Some(3),
            ..SearchLimits::default()
        };
        let control = SearchControl::new();
        let switched_table = TranspositionTable::new(1).unwrap();
        let fresh_table = TranspositionTable::new(1).unwrap();

        let _conservative = search_with_table(
            &position,
            &limits,
            &control,
            SearchSettings::for_test(EvaluationConfig::new(0)),
            &switched_table,
            |_| {},
        );
        let switched = search_with_table(
            &position,
            &limits,
            &control,
            SearchSettings::for_test(EvaluationConfig::new(100)),
            &switched_table,
            |_| {},
        );
        let fresh = search_with_table(
            &position,
            &limits,
            &control,
            SearchSettings::for_test(EvaluationConfig::new(100)),
            &fresh_table,
            |_| {},
        );

        assert_eq!(switched.best_move(), fresh.best_move());
        assert_eq!(
            switched.info().unwrap().score(),
            fresh.info().unwrap().score(),
        );
    }

    #[test]
    fn search_moves_restrict_the_result() {
        let position = Position::default();
        let limits = SearchLimits {
            search_moves: vec!["e2e4".to_owned(), "d2d4".to_owned()],
            ..SearchLimits::default()
        };

        let result = search(&position, &limits);

        assert!(matches!(result.best_move(), Some("d2d4" | "e2e4")));
    }

    #[test]
    fn illegal_search_moves_produce_no_result() {
        let position = Position::default();
        let limits = SearchLimits {
            search_moves: vec!["e2e5".to_owned()],
            ..SearchLimits::default()
        };

        assert_eq!(search(&position, &limits).best_move(), None);
    }

    #[test]
    fn checkmate_has_no_best_move() {
        let position = Position::from_fen("7k/6Q1/6K1/8/8/8/8/8 b - - 0 1").unwrap();

        assert_eq!(
            search(&position, &SearchLimits::default()).best_move(),
            None
        );
    }

    #[test]
    fn stalemate_has_no_best_move() {
        let position = Position::from_fen("7k/5Q2/6K1/8/8/8/8/8 b - - 0 1").unwrap();

        assert_eq!(
            search(&position, &SearchLimits::default()).best_move(),
            None
        );
    }

    #[test]
    fn search_prefers_an_immediate_material_gain() {
        let position = Position::from_fen("7k/8/8/8/8/8/q7/R3K3 w Q - 0 1").unwrap();

        let result = search(&position, &SearchLimits::default());

        assert_eq!(result.best_move(), Some("a1a2"));
    }

    #[test]
    fn reports_each_completed_iteration() {
        let position = Position::default();
        let limits = SearchLimits {
            depth: Some(3),
            ..SearchLimits::default()
        };
        let control = SearchControl::new();
        let mut reports = Vec::new();

        let result = search_with_reporter(&position, &limits, &control, |info| {
            reports.push(info);
        });

        assert_eq!(
            reports.iter().map(SearchInfo::depth).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert!(
            reports
                .windows(2)
                .all(|pair| pair[0].nodes() < pair[1].nodes())
        );
        assert_eq!(result.info(), reports.last());
        assert_eq!(
            result.best_move(),
            reports.last().unwrap().pv().first().map(String::as_str)
        );
        let mut replay = position;
        replay
            .apply_uci_moves(reports.last().unwrap().pv().iter().map(String::as_str))
            .unwrap();
    }

    #[test]
    fn pre_cancelled_search_returns_a_legal_fallback() {
        let position = Position::default();
        let control = SearchControl::new();
        control.stop();

        let result = search_with_reporter(
            &position,
            &SearchLimits {
                depth: Some(20),
                ..SearchLimits::default()
            },
            &control,
            |_| panic!("a cancelled search must not report an iteration"),
        );

        assert!(result.info().is_none());
        assert!(
            position
                .legal_moves()
                .contains(&result.best_move().unwrap().to_owned())
        );
    }

    #[test]
    fn zero_move_time_returns_a_legal_fallback() {
        let position = Position::default();
        let result = search(
            &position,
            &SearchLimits {
                move_time: Some(Duration::ZERO),
                ..SearchLimits::default()
            },
        );

        assert!(result.info().is_none());
        assert!(result.best_move().is_some());
    }

    #[test]
    fn reports_forced_mate_in_moves() {
        let position = Position::from_fen("7k/5Q2/6K1/8/8/8/8/8 w - - 0 1").unwrap();
        let result = search(
            &position,
            &SearchLimits {
                depth: Some(1),
                ..SearchLimits::default()
            },
        );

        assert_eq!(result.info().unwrap().score(), SearchScore::Mate(1));
        let mut checkmate = position;
        checkmate
            .apply_uci_moves([result.best_move().unwrap()])
            .unwrap();
        assert!(checkmate.legal_moves().is_empty());
    }

    #[test]
    fn computes_nodes_per_second_without_dividing_by_zero() {
        let info = SearchInfo::new(
            1,
            SearchScore::Centipawns(0),
            1_000,
            Duration::from_secs(2),
            vec!["e2e4".to_owned()],
        );
        let immediate = SearchInfo::new(
            1,
            SearchScore::Centipawns(0),
            1,
            Duration::ZERO,
            vec!["e2e4".to_owned()],
        );

        assert_eq!(info.nodes_per_second(), 500);
        assert_eq!(immediate.nodes_per_second(), 1_000_000_000);
    }

    #[test]
    fn node_limit_interrupts_an_incomplete_iteration() {
        let position = Position::default();
        let result = search(
            &position,
            &SearchLimits {
                depth: Some(20),
                nodes: Some(1),
                ..SearchLimits::default()
            },
        );

        assert!(result.info().is_none());
        assert!(result.best_move().is_some());
    }

    #[test]
    fn quiescence_avoids_a_poisoned_capture() {
        let position = Position::from_fen("r7/7k/8/8/8/8/p7/Q3K3 w - - 0 1").unwrap();
        let result = search(
            &position,
            &SearchLimits {
                depth: Some(1),
                ..SearchLimits::default()
            },
        );

        assert_ne!(result.best_move(), Some("a1a2"));
    }

    #[test]
    fn root_repetition_is_scored_as_a_draw() {
        let mut position = Position::default();
        position
            .apply_uci_moves([
                "g1f3", "g8f6", "f3g1", "f6g8", "g1f3", "g8f6", "f3g1", "f6g8",
            ])
            .unwrap();

        let result = search(
            &position,
            &SearchLimits {
                depth: Some(4),
                ..SearchLimits::default()
            },
        );

        assert_eq!(result.info().unwrap().depth(), 0);
        assert_eq!(result.info().unwrap().score(), SearchScore::Centipawns(0));
        assert!(result.best_move().is_some());
    }

    #[test]
    fn dead_material_is_scored_as_a_draw() {
        let position = Position::from_fen("7k/8/8/8/8/8/8/KB6 w - - 0 1").unwrap();

        let result = search(
            &position,
            &SearchLimits {
                depth: Some(4),
                ..SearchLimits::default()
            },
        );

        assert_eq!(result.info().unwrap().score(), SearchScore::Centipawns(0));
    }

    #[test]
    fn depth_and_mate_limits_use_the_stricter_bound() {
        let position = Position::default();
        let mut reports = Vec::new();

        search_with_reporter(
            &position,
            &SearchLimits {
                depth: Some(20),
                mate: Some(1),
                ..SearchLimits::default()
            },
            &SearchControl::new(),
            |info| reports.push(info),
        );

        assert_eq!(reports.last().unwrap().depth(), 2);
    }

    #[test]
    fn infinite_search_waits_for_an_explicit_stop() {
        let control = SearchControl::new();
        let worker_control = control.clone();
        let (report_sender, report_receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            search_with_reporter(
                &Position::default(),
                &SearchLimits {
                    infinite: true,
                    ..SearchLimits::default()
                },
                &worker_control,
                |info| report_sender.send(info).unwrap(),
            )
        });

        report_receiver
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        assert!(!worker.is_finished());
        control.stop();
        let result = worker.join().unwrap();

        assert!(result.best_move().is_some());
    }

    #[test]
    fn move_time_interrupts_search_within_a_bounded_delay() {
        let started = std::time::Instant::now();
        let result = search(
            &Position::default(),
            &SearchLimits {
                depth: Some(64),
                move_time: Some(Duration::from_millis(20)),
                ..SearchLimits::default()
            },
        );

        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(result.best_move().is_some());
    }

    #[test]
    fn completed_iterations_respect_the_node_limit() {
        let result = search(
            &Position::default(),
            &SearchLimits {
                depth: Some(20),
                nodes: Some(100),
                ..SearchLimits::default()
            },
        );

        assert!(result.info().unwrap().nodes() <= 100);
    }

    #[test]
    fn depth_one_counts_pvs_researches_as_nodes() {
        let position = Position::default();
        let legal_moves = position.legal_moves().len() as u64;
        let control = SearchControl::new();
        let table = TranspositionTable::new(1).unwrap();
        let result = search_with_table(
            &position,
            &SearchLimits {
                depth: Some(1),
                ..SearchLimits::default()
            },
            &control,
            SearchSettings::for_test(EvaluationConfig::new(0)),
            &table,
            |_| {},
        );

        assert!(result.info().unwrap().nodes() > legal_moves);
    }

    /// A parallel search must still return a usable, self-consistent result.
    ///
    /// Helper searchers make the tree they explore depend on timing, so the
    /// move and score are not pinned. What must hold is that the result is
    /// legal, that its principal variation is playable and starts with the
    /// selected move, and that the reported nodes account for every searcher.
    #[test]
    fn a_parallel_search_returns_a_legal_result_and_playable_pv() {
        let position = Position::from_fen(
            "r1bq1rk1/ppp2ppp/2n2n2/2b1p3/2B1P3/2NP1N2/PPP2PPP/R1BQ1RK1 w - - 0 8",
        )
        .unwrap();
        let table = TranspositionTable::new(4).unwrap();

        for _ in 0..4 {
            let result = search_with_table(
                &position,
                &SearchLimits {
                    depth: Some(6),
                    ..SearchLimits::default()
                },
                &SearchControl::new(),
                SearchSettings {
                    threads: 4,
                    ..SearchSettings::for_test(EvaluationConfig::new(0))
                },
                &table,
                |_| {},
            );

            let best_move = result
                .best_move()
                .expect("a parallel search selects a move");
            assert!(position.legal_moves().contains(&best_move.to_owned()));
            let info = result
                .info()
                .expect("a parallel search completes an iteration");
            assert_eq!(info.pv().first().map(String::as_str), Some(best_move));
            assert!(info.nodes() > 0);
            let mut replay = position.clone();
            replay
                .apply_uci_moves(info.pv().iter().map(String::as_str))
                .expect("a parallel search reports a playable principal variation");
        }
    }

    /// Helper searchers must not outlive the search that spawned them.
    #[test]
    fn a_parallel_search_joins_every_helper_before_returning() {
        let started = std::time::Instant::now();
        let table = TranspositionTable::new(4).unwrap();
        let result = search_with_table(
            &Position::default(),
            &SearchLimits {
                depth: Some(64),
                move_time: Some(Duration::from_millis(50)),
                ..SearchLimits::default()
            },
            &SearchControl::new(),
            SearchSettings {
                threads: 4,
                ..SearchSettings::for_test(EvaluationConfig::new(0))
            },
            &table,
            |_| {},
        );

        assert!(result.best_move().is_some());
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "helpers must be released when the main searcher finishes",
        );
    }

    /// A parallel search must respect the node budget for the search as a whole.
    ///
    /// Each searcher refreshes the shared total only on its polling cadence, so
    /// the budget can be overshot by less than one interval per searcher rather
    /// than by a whole searcher's worth of work.
    #[test]
    fn a_parallel_node_limit_bounds_the_whole_search() {
        const THREADS: usize = 4;
        const LIMIT: u64 = 50_000;

        let table = TranspositionTable::new(4).unwrap();
        let result = search_with_table(
            &Position::default(),
            &SearchLimits {
                depth: Some(64),
                nodes: Some(LIMIT),
                ..SearchLimits::default()
            },
            &SearchControl::new(),
            SearchSettings {
                threads: THREADS,
                ..SearchSettings::for_test(EvaluationConfig::new(0))
            },
            &table,
            |_| {},
        );

        let nodes = result.info().map_or(0, SearchInfo::nodes);
        let allowance = super::algorithm::CONTROL_POLL_INTERVAL_NODES * THREADS as u64;
        assert!(
            nodes <= LIMIT + allowance,
            "{nodes} nodes exceeded {LIMIT} by more than {allowance}",
        );
    }

    /// One configured thread must behave exactly as the search always has.
    #[test]
    fn one_thread_matches_a_search_that_never_spawned_helpers() {
        let position =
            Position::from_fen("r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3")
                .unwrap();
        let limits = SearchLimits {
            depth: Some(6),
            ..SearchLimits::default()
        };

        let first_table = TranspositionTable::new(4).unwrap();
        let first = search_with_table(
            &position,
            &limits,
            &SearchControl::new(),
            SearchSettings::for_test(EvaluationConfig::new(0)),
            &first_table,
            |_| {},
        );
        let second_table = TranspositionTable::new(4).unwrap();
        let second = search_with_table(
            &position,
            &limits,
            &SearchControl::new(),
            SearchSettings::for_test(EvaluationConfig::new(0)),
            &second_table,
            |_| {},
        );

        assert_eq!(first.best_move(), second.best_move());
        assert_eq!(
            first.info().unwrap().score(),
            second.info().unwrap().score()
        );
        assert_eq!(
            first.info().unwrap().nodes(),
            second.info().unwrap().nodes()
        );
    }

    /// Helpers must actually contribute searched work to the shared total.
    ///
    /// This is what distinguishes a parallel search from one that spawns threads
    /// and wastes them: at a fixed time the reported nodes account for every
    /// searcher, so several threads must search more of the tree than one does.
    #[test]
    fn helpers_contribute_nodes_to_the_search_total() {
        let position = Position::from_fen(
            "r1bq1rk1/ppp2ppp/2n2n2/2b1p3/2B1P3/2NP1N2/PPP2PPP/R1BQ1RK1 w - - 0 8",
        )
        .unwrap();
        let limits = SearchLimits {
            depth: Some(64),
            move_time: Some(Duration::from_millis(300)),
            ..SearchLimits::default()
        };
        let nodes = |threads: usize| {
            let table = TranspositionTable::new(8).unwrap();
            search_with_table(
                &position,
                &limits,
                &SearchControl::new(),
                SearchSettings {
                    threads,
                    ..SearchSettings::for_test(EvaluationConfig::new(0))
                },
                &table,
                |_| {},
            )
            .info()
            .map_or(0, SearchInfo::nodes)
        };

        let single = nodes(1);
        let parallel = nodes(4);

        assert!(single > 0, "the single-threaded search reported no nodes");
        assert!(
            parallel > single,
            "four searchers reported {parallel} nodes against {single} for one",
        );
    }

    #[test]
    fn converts_internal_mate_distance_to_uci_moves() {
        assert_eq!(
            SearchScore::from_internal(super::MATE_SCORE - 1),
            SearchScore::Mate(1)
        );
        assert_eq!(
            SearchScore::from_internal(super::MATE_SCORE - 3),
            SearchScore::Mate(2)
        );
        assert_eq!(
            SearchScore::from_internal(-super::MATE_SCORE + 2),
            SearchScore::Mate(-1)
        );
        assert_eq!(SearchScore::from_internal(25), SearchScore::Centipawns(25));
    }

    #[test]
    fn missing_side_to_move_clock_uses_the_default_depth() {
        let mut reports = Vec::new();

        search_with_reporter(
            &Position::default(),
            &SearchLimits {
                black_time: Some(Duration::from_secs(30)),
                ..SearchLimits::default()
            },
            &SearchControl::new(),
            |info| reports.push(info),
        );

        assert_eq!(reports.last().unwrap().depth(), 4);
    }
}
