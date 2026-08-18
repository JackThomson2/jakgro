use cozy_chess::{Color, GameStatus, Piece};

use super::Position;

pub(super) type Score = i32;

pub(super) const NEG_INFINITY: Score = -32_000;
pub(super) const POS_INFINITY: Score = 32_000;
pub(super) const MATE_SCORE: Score = 30_000;

const MATERIAL: [(Piece, Score); 5] = [
    (Piece::Pawn, 100),
    (Piece::Knight, 320),
    (Piece::Bishop, 330),
    (Piece::Rook, 500),
    (Piece::Queen, 900),
];

pub(super) fn evaluate(position: &Position) -> Score {
    let board = position.board();
    match board.status() {
        GameStatus::Won => -MATE_SCORE,
        GameStatus::Drawn => 0,
        GameStatus::Ongoing => {
            let material = MATERIAL
                .iter()
                .map(|&(piece, value)| {
                    let white = board.colored_pieces(Color::White, piece).len() as Score;
                    let black = board.colored_pieces(Color::Black, piece).len() as Score;
                    (white - black) * value
                })
                .sum::<Score>();
            let relative = match board.side_to_move() {
                Color::White => material,
                Color::Black => -material,
            };
            debug_assert!(relative > NEG_INFINITY && relative < POS_INFINITY);
            relative
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MATE_SCORE, evaluate};
    use crate::engine::Position;

    #[test]
    fn starting_material_is_equal() {
        assert_eq!(evaluate(&Position::default()), 0);
    }

    #[test]
    fn material_is_scored_for_the_side_to_move() {
        let white = Position::from_fen("7k/8/8/8/8/8/8/3QK3 w - - 0 1").unwrap();
        let black = Position::from_fen("7k/8/8/8/8/8/8/3QK3 b - - 0 1").unwrap();

        assert_eq!(evaluate(&white), 900);
        assert_eq!(evaluate(&black), -900);
    }

    #[test]
    fn checkmate_uses_a_bounded_terminal_score() {
        let position = Position::from_fen("7k/6Q1/6K1/8/8/8/8/8 b - - 0 1").unwrap();

        assert_eq!(evaluate(&position), -MATE_SCORE);
    }

    #[test]
    fn drawn_positions_are_neutral() {
        let position = Position::from_fen("7k/8/8/8/8/8/8/K7 w - - 0 1").unwrap();

        assert_eq!(evaluate(&position), 0);
    }
}
