//! Protocol-independent engine state and search interfaces.

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::sync::{Arc, Mutex, MutexGuard};

mod evaluation;
mod position;
mod search;

pub use position::{Position, PositionError};
pub use search::{SearchControl, SearchInfo, SearchLimits, SearchResult, SearchScore};

/// Default transposition-table allocation in mebibytes.
pub const DEFAULT_HASH_MIB: usize = search::DEFAULT_HASH_MIB;
/// Smallest accepted transposition-table allocation in mebibytes.
pub const MIN_HASH_MIB: usize = search::MIN_HASH_MIB;
/// Largest accepted transposition-table allocation in mebibytes.
pub const MAX_HASH_MIB: usize = search::MAX_HASH_MIB;

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
#[derive(Clone, Debug)]
pub struct Engine {
    position: Position,
    table: Arc<Mutex<search::TranspositionTable>>,
}

impl Default for Engine {
    fn default() -> Self {
        let table = search::TranspositionTable::new(DEFAULT_HASH_MIB)
            .expect("the default transposition table must be allocatable");
        Self {
            position: Position::default(),
            table: Arc::new(Mutex::new(table)),
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

    /// Resets game state while retaining configured resources.
    pub fn new_game(&mut self) {
        self.position = Position::default();
        self.clear_hash();
    }

    /// Returns the configured transposition-table size in mebibytes.
    #[must_use]
    pub fn hash_size_mib(&self) -> usize {
        self.lock_table().size_mib()
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
        *self.lock_table() = replacement;
        Ok(())
    }

    /// Removes all cached search entries without changing the table size.
    pub fn clear_hash(&self) {
        self.lock_table().clear();
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
        let mut table = self.lock_table();
        search::search_with_table(&self.position, limits, control, &mut table, report)
    }

    /// Computes the normal time budget that should begin after `ponderhit`.
    #[must_use]
    pub(crate) fn ponder_time_budget(&self, limits: &SearchLimits) -> Option<std::time::Duration> {
        search::ponder_time_budget(&self.position, limits)
    }

    fn lock_table(&self) -> MutexGuard<'_, search::TranspositionTable> {
        self.table.lock().unwrap_or_else(|error| error.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::{Engine, Position};

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
}
