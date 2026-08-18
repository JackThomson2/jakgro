use cozy_chess::{Board, Color, Piece};

pub(super) type Score = i32;

pub(super) const NEG_INFINITY: Score = -32_000;
pub(super) const POS_INFINITY: Score = 32_000;
pub(super) const MATE_SCORE: Score = 30_000;
pub(super) const MAX_PLY: u32 = 128;
pub(super) const MATE_THRESHOLD: Score = MATE_SCORE - MAX_PLY as Score;

const MATERIAL: [(Piece, Score); 5] = [
    (Piece::Pawn, 100),
    (Piece::Knight, 320),
    (Piece::Bishop, 330),
    (Piece::Rook, 500),
    (Piece::Queen, 900),
];

pub(super) fn evaluate(board: &Board) -> Score {
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
    use super::evaluate;
    use crate::engine::Position;

    #[test]
    fn starting_material_is_equal() {
        assert_eq!(evaluate(Position::default().board()), 0);
    }

    #[test]
    fn material_is_scored_for_the_side_to_move() {
        let white = Position::from_fen("7k/8/8/8/8/8/8/3QK3 w - - 0 1").unwrap();
        let black = Position::from_fen("7k/8/8/8/8/8/8/3QK3 b - - 0 1").unwrap();

        assert_eq!(evaluate(white.board()), 900);
        assert_eq!(evaluate(black.board()), -900);
    }

    #[test]
    fn material_evaluation_does_not_embed_terminal_scores() {
        let position = Position::from_fen("7k/6Q1/6K1/8/8/8/8/8 b - - 0 1").unwrap();

        assert_eq!(evaluate(position.board()), -900);
    }

    #[test]
    fn drawn_positions_are_neutral() {
        let position = Position::from_fen("7k/8/8/8/8/8/8/K7 w - - 0 1").unwrap();

        assert_eq!(evaluate(position.board()), 0);
    }
}
