use cozy_chess::{
    BitBoard, Board, Move, Piece, Square, get_bishop_moves, get_king_moves, get_knight_moves,
    get_pawn_attacks, get_rook_moves,
};

use crate::engine::evaluation::{Score, piece_value};

#[cfg(test)]
pub(super) fn static_exchange_eval(board: &Board, chess_move: Move) -> Score {
    debug_assert!(board.is_legal(chess_move));
    let mut child = board.clone();
    child.play_unchecked(chess_move);
    static_exchange_eval_after(board, chess_move, &child)
}

pub(super) fn static_exchange_eval_after(board: &Board, chess_move: Move, child: &Board) -> Score {
    move_gain(board, chess_move).map_or(0, |gain| gain - best_capture_gain(child, chess_move.to))
}

fn best_capture_gain(board: &Board, target: Square) -> Score {
    let attackers = attackers_to(board, target);
    let mut best = 0;
    board.generate_moves_for(attackers, |moves| {
        if !moves.to.has(target) {
            return false;
        }
        for chess_move in moves {
            if chess_move.to != target {
                continue;
            }
            let Some(gain) = move_gain(board, chess_move) else {
                continue;
            };
            let mut child = board.clone();
            child.play_unchecked(chess_move);
            best = best.max(gain - best_capture_gain(&child, target));
        }
        false
    });
    best
}

fn attackers_to(board: &Board, target: Square) -> BitBoard {
    let color = board.side_to_move();
    let occupied = board.occupied();
    let diagonal = board.pieces(Piece::Bishop) | board.pieces(Piece::Queen);
    let orthogonal = board.pieces(Piece::Rook) | board.pieces(Piece::Queen);

    board.colored_pieces(color, Piece::Pawn) & get_pawn_attacks(target, !color)
        | board.colored_pieces(color, Piece::Knight) & get_knight_moves(target)
        | board.colors(color) & diagonal & get_bishop_moves(target, occupied)
        | board.colors(color) & orthogonal & get_rook_moves(target, occupied)
        | board.colored_pieces(color, Piece::King) & get_king_moves(target)
}

fn move_gain(board: &Board, chess_move: Move) -> Option<Score> {
    let captured = captured_piece(board, chess_move);
    let promotion_gain = chess_move
        .promotion
        .map(|promotion| piece_value(promotion) - piece_value(Piece::Pawn));
    if captured.is_none() && promotion_gain.is_none() {
        return None;
    }
    Some(captured.map_or(0, piece_value) + promotion_gain.unwrap_or(0))
}

fn captured_piece(board: &Board, chess_move: Move) -> Option<Piece> {
    if board.color_on(chess_move.to) == Some(!board.side_to_move()) {
        return board.piece_on(chess_move.to);
    }
    if board.piece_on(chess_move.from) == Some(Piece::Pawn)
        && board.en_passant() == Some(chess_move.to.file())
        && chess_move.from.file() != chess_move.to.file()
    {
        return Some(Piece::Pawn);
    }
    None
}

#[cfg(test)]
mod tests {
    use cozy_chess::{Board, Color, Move, Piece};

    use super::static_exchange_eval;
    use crate::engine::evaluation::{exchange_outcome, piece_value};

    fn material_balance(board: &Board, perspective: Color) -> i32 {
        let white = [
            Piece::Pawn,
            Piece::Knight,
            Piece::Bishop,
            Piece::Rook,
            Piece::Queen,
        ]
        .into_iter()
        .map(|piece| piece_value(piece) * board.colored_pieces(Color::White, piece).len() as i32)
        .sum::<i32>();
        let black = [
            Piece::Pawn,
            Piece::Knight,
            Piece::Bishop,
            Piece::Rook,
            Piece::Queen,
        ]
        .into_iter()
        .map(|piece| piece_value(piece) * board.colored_pieces(Color::Black, piece).len() as i32)
        .sum::<i32>();
        if perspective == Color::White {
            white - black
        } else {
            black - white
        }
    }

    fn assert_matches_exchange_outcome(fen: &str, move_text: &str) -> i32 {
        let board: Board = fen.parse().unwrap();
        let chess_move: Move = move_text.parse().unwrap();
        assert!(board.is_legal(chess_move));
        let mover = board.side_to_move();
        let before = material_balance(&board, mover);
        let result = static_exchange_eval(&board, chess_move);
        let mut child = board.clone();
        child.play_unchecked(chess_move);
        let outcome = exchange_outcome(&child, mover, chess_move.to);

        assert!(!outcome.truncated);
        assert_eq!(result, outcome.material_balance - before);
        result
    }

    #[test]
    fn legal_see_matches_settled_material_for_common_exchange_shapes() {
        assert_eq!(
            assert_matches_exchange_outcome("4k3/8/8/3q4/8/8/3R4/K7 b - - 0 1", "d5d2",),
            500,
        );
        assert_eq!(
            assert_matches_exchange_outcome("4k3/8/8/3r4/8/8/3Q4/4K3 b - - 0 1", "d5d2",),
            400,
        );
        assert_eq!(
            assert_matches_exchange_outcome("4k3/8/8/8/3pP3/8/8/4K3 b - e3 0 1", "d4e3",),
            100,
        );
        assert_eq!(
            assert_matches_exchange_outcome("4k3/8/8/8/8/8/6p1/4K2R b - - 0 1", "g2h1q",),
            1_300,
        );
        assert_eq!(
            assert_matches_exchange_outcome("4k3/8/8/8/8/8/6p1/4K3 b - - 0 1", "g2g1q",),
            800,
        );
        assert_eq!(
            assert_matches_exchange_outcome("3r3k/8/8/8/3Q4/8/8/3R3K b - - 0 1", "d8d4",),
            400,
        );
    }

    #[test]
    fn legal_see_respects_pins_and_illegal_king_recaptures() {
        assert_eq!(
            assert_matches_exchange_outcome("4k3/4p3/3p4/2Q5/8/8/8/K3R3 w - - 0 1", "c5d6",),
            100,
        );
        assert_eq!(
            assert_matches_exchange_outcome("4k3/8/8/3r2b1/8/8/3Q4/4K3 b - - 0 1", "d5d2",),
            900,
        );
    }
}
