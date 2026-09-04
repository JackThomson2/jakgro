//! Protocol-independent engine state and search interfaces.

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

mod evaluation;
mod position;
mod search;

/// Linear feature vector for offline weight fitting.
///
/// Behind the `tuning` feature, so the shipped engine does not carry it.
#[cfg(feature = "tuning")]
pub use evaluation::tuning;
pub use position::{Position, PositionError};
pub(crate) use search::TimeBudget;
pub use search::{
    SearchControl, SearchInfo, SearchLimits, SearchResult, SearchScore, SearchTelemetry,
};

/// Default transposition-table allocation in mebibytes.
pub const DEFAULT_HASH_MIB: usize = search::DEFAULT_HASH_MIB;
/// Smallest accepted transposition-table allocation in mebibytes.
pub const MIN_HASH_MIB: usize = search::MIN_HASH_MIB;
/// Largest accepted transposition-table allocation in mebibytes.
pub const MAX_HASH_MIB: usize = search::MAX_HASH_MIB;
/// Lowest supported attacking-style percentage.
pub const MIN_AGGRESSION: u8 = evaluation::MIN_AGGRESSION;
/// Default attacking-style percentage.
pub const DEFAULT_AGGRESSION: u8 = evaluation::DEFAULT_AGGRESSION;
/// Highest supported attacking-style percentage.
pub const MAX_AGGRESSION: u8 = evaluation::MAX_AGGRESSION;
/// Lowest supported UCI move-overhead setting in milliseconds.
pub const MIN_MOVE_OVERHEAD_MS: u64 = search::MIN_MOVE_OVERHEAD_MS;
/// Default UCI move-overhead setting in milliseconds.
pub const DEFAULT_MOVE_OVERHEAD_MS: u64 = search::DEFAULT_MOVE_OVERHEAD_MS;
/// Highest supported UCI move-overhead setting in milliseconds.
pub const MAX_MOVE_OVERHEAD_MS: u64 = search::MAX_MOVE_OVERHEAD_MS;
/// Smallest supported number of search threads.
pub const MIN_THREADS: usize = search::MIN_THREADS;
/// Default number of search threads.
pub const DEFAULT_THREADS: usize = search::DEFAULT_THREADS;
/// Largest supported number of search threads.
pub const MAX_THREADS: usize = search::MAX_THREADS;

/// Failure to validate or allocate a requested transposition-table size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HashResizeError {
    /// The requested size falls outside the advertised UCI range.
    OutOfRange { requested: usize },
    /// Memory for the requested table could not be reserved.
    AllocationFailed,
}

impl Display for HashResizeError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfRange { requested } => write!(
                formatter,
                "hash size {requested} MiB is outside {MIN_HASH_MIB}..={MAX_HASH_MIB} MiB",
            ),
            Self::AllocationFailed => {
                formatter.write_str("unable to allocate the requested hash size")
            }
        }
    }
}

impl Error for HashResizeError {}

/// Owns the current game position and coordinates searches.
///
/// The transposition table is shared rather than owned. Its mutex guards
/// replacement alone: a search clones the handle and releases the guard
/// immediately, so searches never serialize against each other. The search
/// memory is shared the same way, so the ordering one search leaves for the
/// next move of the game survives the clone the protocol searches on.
#[derive(Clone, Debug)]
pub struct Engine {
    position: Position,
    evaluation: evaluation::EvaluationConfig,
    move_overhead: Duration,
    threads: usize,
    table: Arc<Mutex<Arc<search::TranspositionTable>>>,
    memory: Arc<Mutex<search::SearchMemory>>,
}

impl Default for Engine {
    fn default() -> Self {
        let table = search::TranspositionTable::new(DEFAULT_HASH_MIB)
            .expect("the default transposition table must be allocatable");
        Self {
            position: Position::default(),
            evaluation: evaluation::EvaluationConfig::default(),
            move_overhead: Duration::from_millis(DEFAULT_MOVE_OVERHEAD_MS),
            threads: DEFAULT_THREADS,
            table: Arc::new(Mutex::new(Arc::new(table))),
            memory: Arc::new(Mutex::new(search::SearchMemory::default())),
        }
    }
}

impl Engine {
    /// Creates an engine at the standard starting position.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the current game position.
    #[must_use]
    pub fn position(&self) -> &Position {
        &self.position
    }

    /// Replaces the current position.
    pub fn set_position(&mut self, position: Position) {
        self.position = position;
    }
    /// Returns the bounded attacking-style percentage used by new searches.
    #[must_use]
    pub fn aggression(&self) -> u8 {
        self.evaluation.aggression()
    }

    /// Changes the attacking-style percentage used by new searches.
    ///
    /// Values above [`MAX_AGGRESSION`] are clamped to that limit.
    pub fn set_aggression(&mut self, aggression: u8) {
        self.evaluation = evaluation::EvaluationConfig::new(aggression);
    }
    /// Returns the time reserved for UCI and operating-system latency.
    #[must_use]
    pub fn move_overhead(&self) -> Duration {
        self.move_overhead
    }

    /// Changes the latency reserve used by clock-managed searches.
    pub fn set_move_overhead(&mut self, move_overhead: Duration) {
        self.move_overhead = move_overhead.min(Duration::from_millis(MAX_MOVE_OVERHEAD_MS));
    }

    /// Returns the number of threads a search may use.
    #[must_use]
    pub fn threads(&self) -> usize {
        self.threads
    }

    /// Changes how many threads a search may use.
    ///
    /// Values outside [`MIN_THREADS`]..=[`MAX_THREADS`] are clamped. One thread
    /// searches deterministically; more than one does not, because the tree the
    /// helpers explore depends on how their timing interleaves.
    pub fn set_threads(&mut self, threads: usize) {
        self.threads = threads.clamp(MIN_THREADS, MAX_THREADS);
    }

    /// Resets game state while retaining configured resources.
    pub fn new_game(&mut self) {
        self.position = Position::default();
        self.clear_hash();
    }

    /// Returns the configured transposition-table size in mebibytes.
    #[must_use]
    pub fn hash_size_mib(&self) -> usize {
        self.shared_table().size_mib()
    }

    /// Replaces the shared transposition table with the requested size.
    pub fn set_hash_size_mib(&self, size_mib: usize) -> Result<(), HashResizeError> {
        if !(MIN_HASH_MIB..=MAX_HASH_MIB).contains(&size_mib) {
            return Err(HashResizeError::OutOfRange {
                requested: size_mib,
            });
        }

        let replacement = search::TranspositionTable::new(size_mib)
            .map_err(|_| HashResizeError::AllocationFailed)?;
        *self.lock_table() = Arc::new(replacement);
        Ok(())
    }

    /// Removes all cached search entries without changing the table size.
    ///
    /// The move ordering the last search left for the next move of its game
    /// is forgotten with them, so a cleared engine searches cold.
    pub fn clear_hash(&self) {
        self.shared_table().clear();
        self.lock_memory().reset();
    }

    /// Whether the last search left its move ordering for the next move.
    #[cfg(test)]
    fn has_search_memory(&self) -> bool {
        self.lock_memory().is_warm()
    }

    /// Searches the current position using the supplied limits.
    #[must_use]
    pub fn search(&self, limits: &SearchLimits) -> SearchResult {
        let control = SearchControl::new();
        self.search_with_reporter(limits, &control, |_| {})
    }

    /// Searches while reporting each completed iterative-deepening result.
    pub fn search_with_reporter<F>(
        &self,
        limits: &SearchLimits,
        control: &SearchControl,
        report: F,
    ) -> SearchResult
    where
        F: FnMut(SearchInfo),
    {
        let table = self.shared_table();
        search::search_with_memory(
            &self.position,
            limits,
            control,
            search::SearchSettings {
                evaluation: self.evaluation,
                move_overhead: self.move_overhead,
                threads: self.threads,
            },
            &table,
            &self.memory,
            report,
        )
    }

    /// Computes the clock budget for a new search.
    #[must_use]
    pub(crate) fn time_budget(&self, limits: &SearchLimits) -> Option<TimeBudget> {
        search::time_budget(&self.position, limits, self.move_overhead)
    }

    /// Computes the normal time budget that should begin after `ponderhit`.
    #[must_use]
    pub(crate) fn ponder_time_budget(&self, limits: &SearchLimits) -> Option<TimeBudget> {
        search::ponder_time_budget(&self.position, limits, self.move_overhead)
    }

    /// Borrows the shared table without holding the replacement guard.
    ///
    /// A search runs for as long as its limits allow, so the guard is released
    /// before the search begins. Resizing and clearing are sequenced against a
    /// running search by the protocol layer, which cancels one before changing
    /// either setting.
    fn shared_table(&self) -> Arc<search::TranspositionTable> {
        Arc::clone(&self.lock_table())
    }

    fn lock_table(&self) -> MutexGuard<'_, Arc<search::TranspositionTable>> {
        self.table.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn lock_memory(&self) -> MutexGuard<'_, search::SearchMemory> {
        self.memory
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_AGGRESSION, DEFAULT_MOVE_OVERHEAD_MS, DEFAULT_THREADS, Engine, MAX_AGGRESSION,
        MAX_MOVE_OVERHEAD_MS, MAX_THREADS, MIN_THREADS, Position,
    };
    use std::time::Duration;

    /// The ordering a search leaves survives into the next move of the same
    /// game and is forgotten with the hash. Which searches may take it is
    /// pinned on the memory itself, in the search module.
    #[test]
    fn search_memory_follows_the_game_and_clears_with_the_hash() {
        let limits = super::SearchLimits {
            nodes: Some(5_000),
            ..super::SearchLimits::default()
        };
        let mut engine = Engine::new();
        assert!(!engine.has_search_memory());
        let _ = engine.search(&limits);
        assert!(engine.has_search_memory());

        let mut continued = Position::default();
        continued.apply_uci_moves(["e2e4", "e7e5"]).unwrap();
        engine.set_position(continued);
        let _ = engine.search(&limits);
        assert!(engine.has_search_memory());

        engine.clear_hash();
        assert!(!engine.has_search_memory());
    }

    #[test]
    fn new_game_restores_the_starting_position() {
        let mut engine = Engine::new();
        let mut position = Position::default();
        position.apply_uci_moves(["e2e4"]).unwrap();
        engine.set_position(position);

        engine.new_game();

        assert_eq!(engine.position(), &Position::default());
    }

    #[test]
    fn hash_resize_is_shared_between_engine_clones_and_validated() {
        let engine = Engine::new();
        let clone = engine.clone();

        engine.set_hash_size_mib(2).unwrap();

        assert_eq!(engine.hash_size_mib(), 2);
        assert_eq!(clone.hash_size_mib(), 2);
        assert!(engine.set_hash_size_mib(0).is_err());
        assert_eq!(engine.hash_size_mib(), 2);
    }

    #[test]
    fn new_game_preserves_the_configured_hash_size() {
        let mut engine = Engine::new();
        engine.set_hash_size_mib(2).unwrap();

        engine.new_game();

        assert_eq!(engine.hash_size_mib(), 2);
    }
    #[test]
    fn aggression_is_clamped_and_preserved_across_new_games() {
        let mut engine = Engine::new();
        assert_eq!(engine.aggression(), DEFAULT_AGGRESSION);

        engine.set_aggression(37);
        let clone = engine.clone();
        engine.new_game();

        assert_eq!(engine.aggression(), 37);
        assert_eq!(clone.aggression(), 37);
        engine.set_aggression(u8::MAX);
        assert_eq!(engine.aggression(), MAX_AGGRESSION);
    }
    #[test]
    fn threads_are_clamped_and_preserved_across_new_games() {
        let mut engine = Engine::new();
        assert_eq!(engine.threads(), DEFAULT_THREADS);

        engine.set_threads(4);
        let clone = engine.clone();
        engine.new_game();

        assert_eq!(engine.threads(), 4);
        assert_eq!(clone.threads(), 4);
        engine.set_threads(0);
        assert_eq!(engine.threads(), MIN_THREADS);
        engine.set_threads(usize::MAX);
        assert_eq!(engine.threads(), MAX_THREADS);
    }

    #[test]
    fn move_overhead_is_clamped_and_preserved_across_new_games() {
        let mut engine = Engine::new();
        assert_eq!(
            engine.move_overhead(),
            Duration::from_millis(DEFAULT_MOVE_OVERHEAD_MS),
        );

        engine.set_move_overhead(Duration::from_millis(250));
        let clone = engine.clone();
        engine.new_game();

        assert_eq!(engine.move_overhead(), Duration::from_millis(250));
        assert_eq!(clone.move_overhead(), Duration::from_millis(250));
        engine.set_move_overhead(Duration::MAX);
        assert_eq!(
            engine.move_overhead(),
            Duration::from_millis(MAX_MOVE_OVERHEAD_MS),
        );
    }
}
