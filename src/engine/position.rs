use std::error::Error;
use std::fmt::{self, Display, Formatter};

use cozy_chess::util::{display_uci_move, parse_uci_move};
use cozy_chess::{Board, Move};

/// A legal standard-chess position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Position {
    board: Board,
}

impl Position {
    /// Creates the standard starting position.
    #[must_use]
    pub fn start_position() -> Self {
        Self {
            board: Board::default(),
        }
    }

    /// Parses a six-field Forsyth-Edwards Notation position.
    pub fn from_fen(fen: &str) -> Result<Self, PositionError> {
        fen.parse::<Board>()
            .map(|board| Self { board })
            .map_err(|_| PositionError::InvalidFen(fen.to_owned()))
    }

    /// Applies UCI moves without changing this position if any move is invalid.
    pub fn apply_uci_moves<I, S>(&mut self, moves: I) -> Result<(), PositionError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut candidate = self.board.clone();

        for (index, move_text) in moves.into_iter().enumerate() {
            let move_text = move_text.as_ref();
            let chess_move = parse_uci_move(&candidate, move_text).map_err(|_| {
                PositionError::MalformedMove {
                    ply: index + 1,
                    move_text: move_text.to_owned(),
                }
            })?;

            candidate
                .try_play(chess_move)
                .map_err(|_| PositionError::IllegalMove {
                    ply: index + 1,
                    move_text: move_text.to_owned(),
                })?;
        }

        self.board = candidate;
        Ok(())
    }

    /// Returns every legal move in standard UCI notation and deterministic order.
    #[must_use]
    pub fn legal_moves(&self) -> Vec<String> {
        let mut moves = self.internal_legal_moves();
        let mut uci_moves = moves
            .drain(..)
            .map(|chess_move| display_uci_move(&self.board, chess_move).to_string())
            .collect::<Vec<_>>();
        uci_moves.sort_unstable();
        uci_moves
    }

    fn internal_legal_moves(&self) -> Vec<Move> {
        let mut moves = Vec::new();
        self.board.generate_moves(|piece_moves| {
            moves.extend(piece_moves);
            false
        });
        moves
    }
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
}
