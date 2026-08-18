use std::error::Error;
use std::fmt::{self, Display, Formatter};

use cozy_chess::util::{display_uci_move, parse_uci_move};
use cozy_chess::{Board, Move, Rank, Square, get_pawn_attacks};

/// A legal standard-chess position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Position {
    board: Board,
    hash_history: Vec<u64>,
}

impl Position {
    /// Creates the standard starting position.
    #[must_use]
    pub fn start_position() -> Self {
        Self::from_board(Board::default())
    }

    /// Parses a six-field Forsyth-Edwards Notation position.
    pub fn from_fen(fen: &str) -> Result<Self, PositionError> {
        fen.parse::<Board>()
            .map(Self::from_board)
            .map_err(|_| PositionError::InvalidFen(fen.to_owned()))
    }

    /// Applies UCI moves without changing this position if any move is invalid.
    pub fn apply_uci_moves<I, S>(&mut self, moves: I) -> Result<(), PositionError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut candidate = self.clone();

        for (index, move_text) in moves.into_iter().enumerate() {
            let move_text = move_text.as_ref();
            let chess_move = parse_uci_move(&candidate.board, move_text).map_err(|_| {
                PositionError::MalformedMove {
                    ply: index + 1,
                    move_text: move_text.to_owned(),
                }
            })?;

            candidate
                .board
                .try_play(chess_move)
                .map_err(|_| PositionError::IllegalMove {
                    ply: index + 1,
                    move_text: move_text.to_owned(),
                })?;
            candidate
                .hash_history
                .push(repetition_key(&candidate.board));
        }

        *self = candidate;
        Ok(())
    }

    /// Returns every legal move in standard UCI notation and deterministic order.
    #[must_use]
    pub fn legal_moves(&self) -> Vec<String> {
        let mut uci_moves = self
            .search_moves()
            .into_iter()
            .map(|chess_move| self.format_search_move(chess_move))
            .collect::<Vec<_>>();
        uci_moves.sort_unstable();
        uci_moves
    }

    pub(super) fn search_moves(&self) -> Vec<Move> {
        let mut moves = Vec::new();
        self.board.generate_moves(|piece_moves| {
            moves.extend(piece_moves);
            false
        });
        moves
    }

    pub(super) fn format_search_move(&self, chess_move: Move) -> String {
        display_uci_move(&self.board, chess_move).to_string()
    }

    pub(super) fn play_search_move(&self, chess_move: Move) -> Self {
        debug_assert!(self.board.is_legal(chess_move));
        let mut child = self.clone();
        child.board.play_unchecked(chess_move);
        child.hash_history.push(repetition_key(&child.board));
        child
    }

    pub(super) fn board(&self) -> &Board {
        &self.board
    }

    pub(super) fn hash_history(&self) -> &[u64] {
        &self.hash_history
    }

    /// Returns how often the current position occurs in this game's history.
    #[must_use]
    pub fn repetition_count(&self) -> usize {
        let current = repetition_key(&self.board);
        self.hash_history()
            .iter()
            .filter(|&&hash| hash == current)
            .count()
    }

    /// Returns whether the current position has occurred at least three times.
    #[must_use]
    pub fn is_threefold_repetition(&self) -> bool {
        self.repetition_count() >= 3
    }

    fn from_board(board: Board) -> Self {
        let hash_history = vec![repetition_key(&board)];
        Self {
            board,
            hash_history,
        }
    }
}

fn repetition_key(board: &Board) -> u64 {
    if let Some(file) = board.en_passant() {
        let color = board.side_to_move();
        let target = Square::new(file, Rank::Sixth.relative_to(color));
        for from in get_pawn_attacks(target, !color) {
            let chess_move = Move {
                from,
                to: target,
                promotion: None,
            };
            if board.is_legal(chess_move) {
                return board.hash();
            }
        }
    }

    board.hash_without_ep()
}

impl Default for Position {
    fn default() -> Self {
        Self::start_position()
    }
}

impl Display for Position {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.board, formatter)
    }
}

/// An error encountered while constructing a legal position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PositionError {
    /// The supplied FEN could not be parsed or represented a malformed position.
    InvalidFen(String),
    /// A move was not valid UCI coordinate notation.
    MalformedMove {
        /// One-based index within the supplied move list.
        ply: usize,
        /// The invalid move text.
        move_text: String,
    },
    /// A syntactically valid move was illegal in its position.
    IllegalMove {
        /// One-based index within the supplied move list.
        ply: usize,
        /// The illegal move text.
        move_text: String,
    },
}

impl Display for PositionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFen(fen) => write!(formatter, "invalid FEN: {fen}"),
            Self::MalformedMove { ply, move_text } => {
                write!(formatter, "malformed move at ply {ply}: {move_text}")
            }
            Self::IllegalMove { ply, move_text } => {
                write!(formatter, "illegal move at ply {ply}: {move_text}")
            }
        }
    }
}

impl Error for PositionError {}

#[cfg(test)]
mod tests {
    use super::{Position, PositionError};

    #[test]
    fn start_position_has_twenty_legal_moves() {
        assert_eq!(Position::default().legal_moves().len(), 20);
    }

    #[test]
    fn parses_and_displays_fen() {
        const FEN: &str = "4k3/8/8/8/8/8/8/4K3 w - - 0 1";

        let position = Position::from_fen(FEN).unwrap();

        assert_eq!(position.to_string(), FEN);
    }

    #[test]
    fn applies_castling_in_standard_uci_notation() {
        let mut position = Position::from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").unwrap();

        assert!(position.legal_moves().contains(&"e1g1".to_owned()));
        position.apply_uci_moves(["e1g1"]).unwrap();

        assert_eq!(position.to_string(), "r3k2r/8/8/8/8/8/8/R4RK1 b kq - 1 1");
    }

    #[test]
    fn applies_promotion() {
        let mut position = Position::from_fen("7k/P7/8/8/8/8/8/4K3 w - - 0 1").unwrap();

        position.apply_uci_moves(["a7a8q"]).unwrap();

        assert_eq!(position.to_string(), "Q6k/8/8/8/8/8/8/4K3 b - - 0 1");
    }

    #[test]
    fn applies_en_passant_capture() {
        let mut position = Position::from_fen("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 2").unwrap();

        position.apply_uci_moves(["e5d6"]).unwrap();

        assert_eq!(position.to_string(), "4k3/8/3P4/8/8/8/8/4K3 b - - 0 2");
    }

    #[test]
    fn rejects_invalid_fen() {
        assert!(matches!(
            Position::from_fen("not a fen"),
            Err(PositionError::InvalidFen(_))
        ));
    }

    #[test]
    fn move_lists_are_transactional() {
        let mut position = Position::default();
        let original = position.clone();

        let error = position.apply_uci_moves(["e2e4", "e7e5", "e2e3"]);

        assert!(matches!(
            error,
            Err(PositionError::IllegalMove { ply: 3, .. })
        ));
        assert_eq!(position, original);
    }

    #[test]
    fn distinguishes_malformed_moves() {
        let error = Position::default().apply_uci_moves(["e2-e4"]);

        assert!(matches!(
            error,
            Err(PositionError::MalformedMove { ply: 1, .. })
        ));
    }

    #[test]
    fn records_threefold_repetition_history() {
        let mut position = Position::default();
        position
            .apply_uci_moves([
                "g1f3", "g8f6", "f3g1", "f6g8", "g1f3", "g8f6", "f3g1", "f6g8",
            ])
            .unwrap();

        assert_eq!(position.repetition_count(), 3);
        assert!(position.is_threefold_repetition());
    }

    #[test]
    fn ignores_ineffective_en_passant_in_repetition_keys() {
        let with_ep = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1"
            .parse::<cozy_chess::Board>()
            .unwrap();
        let without_ep = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1"
            .parse::<cozy_chess::Board>()
            .unwrap();

        assert_eq!(
            super::repetition_key(&with_ep),
            super::repetition_key(&without_ep)
        );
    }

    #[test]
    fn preserves_effective_en_passant_in_repetition_keys() {
        let with_ep = "4k3/8/8/8/3pP3/8/8/4K3 b - e3 0 1"
            .parse::<cozy_chess::Board>()
            .unwrap();
        let without_ep = "4k3/8/8/8/3pP3/8/8/4K3 b - - 0 1"
            .parse::<cozy_chess::Board>()
            .unwrap();

        assert_ne!(
            super::repetition_key(&with_ep),
            super::repetition_key(&without_ep)
        );
    }

    #[test]
    fn search_children_do_not_change_the_parent() {
        let position = Position::default();
        let chess_move = position
            .search_moves()
            .into_iter()
            .find(|&chess_move| position.format_search_move(chess_move) == "e2e4")
            .unwrap();

        let child = position.play_search_move(chess_move);

        assert_eq!(position, Position::default());
        assert_eq!(child.hash_history().len(), 2);
        assert_eq!(
            child.to_string(),
            "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1"
        );
    }
}
