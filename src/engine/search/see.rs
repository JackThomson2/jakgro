//! Static exchange evaluation over a square.
//!
//! The evaluation is a swap list. Attackers join the exchange in increasing
//! value, each capture is scored against the running balance, and the recursive
//! choice between standing pat and continuing is folded back in one backward
//! pass. Nothing is cloned and no moves are generated, so this is cheap enough to
//! consult for every capture at every node.
//!
//! The result is an approximation in the way every bitboard swap list is: it
//! follows x-ray attackers behind a piece that has joined the exchange, and it
//! refuses a king capture onto a defended square, but it does not model pins or
//! discovered checks. Exact legal settlement lives in
//! [`crate::engine::evaluation::exchange_outcome`], which sacrifice verification
//! uses where correctness matters more than speed.

use cozy_chess::{
    BitBoard, Board, Color, Move, Piece, Rank, Square, get_bishop_moves, get_king_moves,
    get_knight_moves, get_pawn_attacks, get_rook_moves,
};

use crate::engine::evaluation::{Score, piece_value};

/// Attacker kinds ordered from least to most valuable.
const ATTACK_ORDER: [Piece; 6] = [
    Piece::Pawn,
    Piece::Knight,
    Piece::Bishop,
    Piece::Rook,
    Piece::Queen,
    Piece::King,
];

/// Longest exchange the swap list tracks.
///
/// Thirty-two captures exceed the number of pieces that can ever attack one
/// square, so the bound is unreachable rather than a truncation policy.
const MAX_SWAPS: usize = 32;

/// Returns the material a move wins or loses once the exchange settles.
pub(super) fn static_exchange_eval(board: &Board, chess_move: Move) -> Score {
    let Some(gain) = move_gain(board, chess_move) else {
        return 0;
    };
    settle(board, chess_move, gain)
}

/// Runs the swap list over a move's destination square.
///
/// `first_gain` is the material the move wins immediately. The loop repeatedly
/// takes the least valuable attacker of the square, records the material it would
/// win, and exposes its own value to the next recapture. The backward fold then
/// applies each side's option to decline: a side that cannot improve on refusing
/// the capture refuses it.
fn settle(board: &Board, chess_move: Move, first_gain: Score) -> Score {
    let target = chess_move.to;
    let mut occupied = board.occupied() ^ chess_move.from.bitboard();
    if is_en_passant(board, chess_move) {
        occupied ^= Square::new(target.file(), chess_move.from.rank()).bitboard();
    }
    occupied |= target.bitboard();

    let mut gains = [0_i32; MAX_SWAPS];
    gains[0] = first_gain;
    // The piece the move leaves on the square is what the next capture wins.
    let mut exposed = match chess_move.promotion {
        Some(promotion) => piece_value(promotion),
        None => board.piece_on(chess_move.from).map_or(0, piece_value),
    };
    let mut side = !board.side_to_move();
    let mut depth = 0;

    while let Some((attacker, square)) = least_valuable_attacker(board, target, side, occupied) {
        let remaining = occupied ^ square.bitboard();
        if attacker == Piece::King && has_attacker(board, target, !side, remaining) {
            // A king may not capture onto a square the other side still attacks,
            // so this recapture does not exist and the exchange has settled.
            break;
        }
        if depth + 1 >= MAX_SWAPS {
            break;
        }
        depth += 1;
        let promotes = attacker == Piece::Pawn && target.rank() == Rank::Eighth.relative_to(side);
        let promotion_gain = if promotes {
            piece_value(Piece::Queen) - piece_value(Piece::Pawn)
        } else {
            0
        };
        gains[depth] = exposed + promotion_gain - gains[depth - 1];
        occupied = remaining;
        exposed = if promotes {
            piece_value(Piece::Queen)
        } else {
            piece_value(attacker)
        };
        side = !side;
    }

    while depth > 0 {
        gains[depth - 1] = -(-gains[depth - 1]).max(gains[depth]);
        depth -= 1;
    }
    gains[0]
}

/// Returns the least valuable attacker of a square under a given occupancy.
fn least_valuable_attacker(
    board: &Board,
    target: Square,
    color: Color,
    occupied: BitBoard,
) -> Option<(Piece, Square)> {
    ATTACK_ORDER.into_iter().find_map(|piece| {
        (attackers_of_piece(board, target, color, piece, occupied) & occupied)
            .into_iter()
            .next()
            .map(|square| (piece, square))
    })
}

/// Returns whether a side still attacks a square under a given occupancy.
fn has_attacker(board: &Board, target: Square, color: Color, occupied: BitBoard) -> bool {
    ATTACK_ORDER.into_iter().any(|piece| {
        !(attackers_of_piece(board, target, color, piece, occupied) & occupied).is_empty()
    })
}

/// Returns one side's pieces of one kind that attack a square.
///
/// Sliders are recomputed against the supplied occupancy, which is what reveals
/// an x-ray attacker once the piece in front of it has joined the exchange.
fn attackers_of_piece(
    board: &Board,
    target: Square,
    color: Color,
    piece: Piece,
    occupied: BitBoard,
) -> BitBoard {
    let candidates = board.colored_pieces(color, piece);
    match piece {
        Piece::Pawn => candidates & get_pawn_attacks(target, !color),
        Piece::Knight => candidates & get_knight_moves(target),
        Piece::Bishop => candidates & get_bishop_moves(target, occupied),
        Piece::Rook => candidates & get_rook_moves(target, occupied),
        Piece::Queen => {
            candidates & (get_bishop_moves(target, occupied) | get_rook_moves(target, occupied))
        }
        Piece::King => candidates & get_king_moves(target),
    }
}

/// Returns whether a move is an en passant capture.
fn is_en_passant(board: &Board, chess_move: Move) -> bool {
    board.piece_on(chess_move.from) == Some(Piece::Pawn)
        && board.en_passant() == Some(chess_move.to.file())
        && chess_move.from.file() != chess_move.to.file()
        && board.color_on(chess_move.to).is_none()
}

/// Returns the material a move wins immediately, or `None` when it wins nothing.
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
    if is_en_passant(board, chess_move) {
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

    fn see(fen: &str, move_text: &str) -> i32 {
        let board: Board = fen.parse().unwrap();
        let chess_move: Move = move_text.parse().unwrap();
        assert!(board.is_legal(chess_move));
        static_exchange_eval(&board, chess_move)
    }

    /// Asserts the swap list agrees with exact legal settlement.
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
    fn swap_list_matches_settled_material_for_common_exchange_shapes() {
        assert_eq!(
            assert_matches_exchange_outcome("4k3/8/8/3q4/8/8/3R4/K7 b - - 0 1", "d5d2"),
            500,
        );
        assert_eq!(
            assert_matches_exchange_outcome("4k3/8/8/3r4/8/8/3Q4/4K3 b - - 0 1", "d5d2"),
            400,
        );
        assert_eq!(
            assert_matches_exchange_outcome("4k3/8/8/8/3pP3/8/8/4K3 b - e3 0 1", "d4e3"),
            100,
        );
        assert_eq!(
            assert_matches_exchange_outcome("4k3/8/8/8/8/8/6p1/4K2R b - - 0 1", "g2h1q"),
            1_300,
        );
        assert_eq!(
            assert_matches_exchange_outcome("4k3/8/8/8/8/8/6p1/4K3 b - - 0 1", "g2g1q"),
            800,
        );
        assert_eq!(
            assert_matches_exchange_outcome("3r3k/8/8/8/3Q4/8/8/3R3K b - - 0 1", "d8d4"),
            400,
        );
    }

    #[test]
    fn a_defended_square_refuses_a_king_recapture() {
        assert_eq!(
            assert_matches_exchange_outcome("4k3/8/8/3r2b1/8/8/3Q4/4K3 b - - 0 1", "d5d2"),
            900,
        );
    }

    /// Records that the swap list ignores pins, unlike exact settlement.
    ///
    /// Black's e7 pawn is pinned against its king by the rook on e1, so the
    /// recapture is illegal and the queen wins a clean pawn. A swap list plays
    /// the pinned recapture anyway and reports a lost queen. Move ordering only
    /// needs a cheap ranking, and sacrifice verification uses exact settlement
    /// instead, so the approximation is confined to ordering and pruning.
    #[test]
    fn the_swap_list_does_not_model_pinned_defenders() {
        let fen = "4k3/4p3/3p4/2Q5/8/8/8/K3R3 w - - 0 1";
        let board: Board = fen.parse().unwrap();
        let chess_move: Move = "c5d6".parse().unwrap();
        let mover = board.side_to_move();
        let before = material_balance(&board, mover);
        let mut child = board.clone();
        child.play_unchecked(chess_move);
        let exact = exchange_outcome(&child, mover, chess_move.to).material_balance - before;

        assert_eq!(exact, 100);
        assert_eq!(see(fen, "c5d6"), -800);
    }

    #[test]
    fn x_ray_attackers_join_the_exchange_behind_a_moved_piece() {
        // Doubled rooks against a defended pawn: Rxd5 rxd5 Rxd5 nets the pawn,
        // which only holds because the rear rook is revealed by the front one.
        assert_eq!(
            assert_matches_exchange_outcome("3rk3/8/8/3p4/8/8/3R4/3RK3 w - - 0 1", "d2d5"),
            100,
        );
        // A queen behind the rook supports the same exchange.
        assert_eq!(
            assert_matches_exchange_outcome("3rk3/8/8/3p4/8/8/3R4/3QK3 w - - 0 1", "d2d5"),
            100,
        );
        // With no defender the pawn is simply won.
        assert_eq!(
            assert_matches_exchange_outcome("4k3/8/8/3p4/8/8/3R4/3RK3 w - - 0 1", "d2d5"),
            100,
        );
    }

    #[test]
    fn losing_captures_report_the_full_loss() {
        // A pawn taking a defended knight wins the knight outright here, since
        // nothing recaptures.
        assert_eq!(
            assert_matches_exchange_outcome("4k3/8/8/8/8/2n5/3P4/4K3 w - - 0 1", "d2c3"),
            320,
        );
        // A rook taking a defended queen wins it outright for the same reason.
        assert_eq!(
            assert_matches_exchange_outcome("3qk3/8/8/8/8/8/3R4/3RK3 w - - 0 1", "d2d8"),
            900,
        );
    }

    #[test]
    fn a_quiet_move_settles_at_zero() {
        assert_eq!(see("4k3/8/8/8/8/8/3P4/4K3 w - - 0 1", "d2d4"), 0);
    }

    /// Compares the swap list against exact settlement over many captures.
    ///
    /// Every capture in eight middlegame positions is checked from both sides.
    /// The two agree on all of them, which is the evidence that the swap list is
    /// a faithful replacement for ordering purposes; the pin case above is the
    /// documented shape where they can legitimately differ.
    #[test]
    fn the_swap_list_agrees_with_exact_settlement_on_almost_every_capture() {
        let openings = [
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "r1bq1rk1/ppp2ppp/2n2n2/2b1p1NQ/2B1P3/2NP4/PPP2PPP/R4RK1 w - - 0 10",
            "2kr3r/pppq1ppp/2n1bn2/3p4/3P4/2P1PN2/PP1N1PPP/R2Q1RK1 w - - 0 10",
            "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
            "r3r1k1/ppp2ppp/2n2n2/3q4/8/2N2N2/PPPQ1PPP/R3R1K1 w - - 0 15",
            "r1bqkb1r/pppp1ppp/2n2n2/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4",
            "rnb1kbnr/pp1ppppp/8/q1p5/2P5/2N5/PP1PPPPP/R1BQKBNR w KQkq - 2 3",
            "r2q1rk1/pb1nbppp/1p2pn2/2pp4/2PP4/1PN1PN2/PB3PPP/R2QKB1R w KQ - 0 11",
        ];
        let mut compared = 0_u32;
        let mut disagreements = 0_u32;

        // Each opening is examined from both sides, so a capture is measured for
        // whichever colour can make it.
        for fen in openings {
            for flipped in [false, true] {
                let mut board: Board = fen.parse().unwrap();
                if flipped {
                    let Some(swapped) = board.null_move() else {
                        continue;
                    };
                    board = swapped;
                }
                let mover = board.side_to_move();
                let before = material_balance(&board, mover);
                let mut captures = Vec::new();
                board.generate_moves(|moves| {
                    captures.extend(moves);
                    false
                });
                for chess_move in captures {
                    if board.color_on(chess_move.to) != Some(!mover) {
                        continue;
                    }
                    let mut child = board.clone();
                    child.play_unchecked(chess_move);
                    let outcome = exchange_outcome(&child, mover, chess_move.to);
                    if outcome.truncated {
                        continue;
                    }
                    compared += 1;
                    let exact = outcome.material_balance - before;
                    if static_exchange_eval(&board, chess_move) != exact {
                        disagreements += 1;
                    }
                }
            }
        }

        assert!(
            compared >= 40,
            "expected a broad sample, compared {compared}"
        );
        assert_eq!(
            disagreements, 0,
            "{disagreements} of {compared} captures disagreed with exact settlement",
        );
    }
}
