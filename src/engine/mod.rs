//! Protocol-independent engine state and search interfaces.

mod evaluation;
mod position;
mod search;

pub use position::{Position, PositionError};
pub use search::{SearchControl, SearchInfo, SearchLimits, SearchResult, SearchScore};

/// Owns the current game position and coordinates searches.
#[derive(Clone, Debug, Default)]
pub struct Engine {
    position: Position,
}

impl Engine {
    /// Creates an engine at the standard starting position.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the engine's current position.
    #[must_use]
    pub fn position(&self) -> &Position {
        &self.position
    }

    /// Replaces the engine's current position.
    pub fn set_position(&mut self, position: Position) {
        self.position = position;
    }

    /// Resets all game state to the standard starting position.
    pub fn new_game(&mut self) {
        self.position = Position::default();
    }

    /// Searches the current position within the supplied limits.
    #[must_use]
    pub fn search(&self, limits: &SearchLimits) -> SearchResult {
        search::search(&self.position, limits)
    }

    /// Searches while reporting every fully completed iteration.
    #[must_use]
    pub fn search_with_reporter<F>(
        &self,
        limits: &SearchLimits,
        control: &SearchControl,
        report: F,
    ) -> SearchResult
    where
        F: FnMut(SearchInfo),
    {
        search::search_with_reporter(&self.position, limits, control, report)
    }
    pub(crate) fn ponder_time_budget(&self, limits: &SearchLimits) -> Option<std::time::Duration> {
        search::ponder_time_budget(&self.position, limits)
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
}
