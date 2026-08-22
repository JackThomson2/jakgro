mod algorithm;
mod control;
mod time;
mod transposition;

use std::time::Duration;

pub use control::SearchControl;
pub(crate) use time::TimeBudget;
pub(super) use time::{DEFAULT_MOVE_OVERHEAD_MS, MAX_MOVE_OVERHEAD_MS, MIN_MOVE_OVERHEAD_MS};
pub(super) use transposition::{DEFAULT_HASH_MIB, MAX_HASH_MIB, MIN_HASH_MIB, TranspositionTable};

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

/// The principal result produced by a search.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchResult {
    best_move: Option<String>,
    ponder: Option<String>,
    info: Option<SearchInfo>,
}

impl SearchResult {
    fn from_parts(best_move: Option<String>, info: Option<SearchInfo>) -> Self {
        let ponder = info
            .as_ref()
            .and_then(|search_info| search_info.pv().get(1).cloned());
        Self {
            best_move,
            ponder,
            info,
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
    let mut table = TranspositionTable::new(MIN_HASH_MIB)
        .expect("the minimum transposition table must be allocatable");
    search_with_table(
        position,
        limits,
        control,
        EvaluationConfig::default(),
        Duration::from_millis(DEFAULT_MOVE_OVERHEAD_MS),
        &mut table,
        report,
    )
}

pub(super) fn search_with_table<F>(
    position: &Position,
    limits: &SearchLimits,
    control: &SearchControl,
    evaluation: EvaluationConfig,
    move_overhead: Duration,
    table: &mut TranspositionTable,
    report: F,
) -> SearchResult
where
    F: FnMut(SearchInfo),
{
    algorithm::run(
        position,
        limits,
        control,
        evaluation,
        move_overhead,
        table,
        report,
    )
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
        DEFAULT_MOVE_OVERHEAD_MS, EvaluationConfig, SearchControl, SearchInfo, SearchLimits,
        SearchScore, TranspositionTable, search, search_with_reporter, search_with_table,
    };
    use crate::engine::Position;
    const MOVE_OVERHEAD: Duration = Duration::from_millis(DEFAULT_MOVE_OVERHEAD_MS);

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
        let mut table = TranspositionTable::new(1).unwrap();

        let cold = search_with_table(
            &position,
            &limits,
            &control,
            EvaluationConfig::default(),
            MOVE_OVERHEAD,
            &mut table,
            |_| {},
        );
        let warm = search_with_table(
            &position,
            &limits,
            &control,
            EvaluationConfig::default(),
            MOVE_OVERHEAD,
            &mut table,
            |_| {},
        );

        assert_eq!(warm.best_move(), cold.best_move());
        assert_eq!(warm.info().unwrap().score(), cold.info().unwrap().score());
        assert!(warm.info().unwrap().nodes() < cold.info().unwrap().nodes());
    }
    #[test]
    fn aggression_changes_do_not_reuse_stale_hash_scores() {
        let position =
            Position::from_fen("rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2")
                .unwrap();
        let limits = SearchLimits {
            depth: Some(3),
            ..SearchLimits::default()
        };
        let control = SearchControl::new();
        let mut switched_table = TranspositionTable::new(1).unwrap();
        let mut fresh_table = TranspositionTable::new(1).unwrap();

        let conservative = search_with_table(
            &position,
            &limits,
            &control,
            EvaluationConfig::new(0),
            MOVE_OVERHEAD,
            &mut switched_table,
            |_| {},
        );
        let switched = search_with_table(
            &position,
            &limits,
            &control,
            EvaluationConfig::new(100),
            MOVE_OVERHEAD,
            &mut switched_table,
            |_| {},
        );
        let fresh = search_with_table(
            &position,
            &limits,
            &control,
            EvaluationConfig::new(100),
            MOVE_OVERHEAD,
            &mut fresh_table,
            |_| {},
        );

        assert_ne!(
            conservative.info().unwrap().score(),
            switched.info().unwrap().score(),
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
        let control = SearchControl::new();
        let mut table = TranspositionTable::new(1).unwrap();
        let result = search_with_table(
            &Position::default(),
            &SearchLimits {
                depth: Some(1),
                ..SearchLimits::default()
            },
            &control,
            EvaluationConfig::new(0),
            MOVE_OVERHEAD,
            &mut table,
            |_| {},
        );

        assert_eq!(result.info().unwrap().nodes(), 24);
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
