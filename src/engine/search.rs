use std::time::Duration;

use super::Position;

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

/// The principal result produced by a search.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchResult {
    best_move: Option<String>,
    ponder: Option<String>,
}

impl SearchResult {
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
}

pub(super) fn search(position: &Position, limits: &SearchLimits) -> SearchResult {
    let mut legal_moves = position.legal_moves();

    if !limits.search_moves.is_empty() {
        legal_moves.retain(|legal_move| {
            limits
                .search_moves
                .iter()
                .any(|candidate| candidate == legal_move)
        });
    }

    SearchResult {
        best_move: legal_moves.into_iter().next(),
        ponder: None,
    }
}

#[cfg(test)]
mod tests {
    use super::{SearchLimits, search};
    use crate::engine::Position;

    #[test]
    fn baseline_search_is_deterministic() {
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
}
