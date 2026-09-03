//! Endgame scaling for material the stronger side cannot convert.
//!
//! A linear evaluation scores a bishop up as a bishop up whatever else is on
//! the board. Some of those positions are won and some are dead: a lone minor
//! piece mates nothing, opposite-coloured bishops hold with two pawns down,
//! and a side with no pawns left needs more than a piece to win. The fit
//! cannot express any of that, because none of it is a sum of terms — it is a
//! multiplier on the whole ending. These rules supply the multiplier, and
//! they are hand-set and measured by match rather than fitted.

use cozy_chess::{BitBoard, Board, Color, Piece};

use super::{Score, piece_value};

/// The scale at which the endgame component counts in full.
pub(super) const FULL: Score = 64;
/// A side whose material cannot mate.
const DEAD: Score = 0;
/// A pawnless advantage of a minor piece or less.
const BARE_MINOR: Score = 8;
/// A pawnless advantage above a minor piece but below a rook.
const PAWNLESS_PIECE: Score = 32;
/// Opposite-coloured bishops with nothing else, before their pawns count.
const OPPOSITE_BISHOPS: Score = 24;
/// What each of the stronger side's pawns adds under opposite bishops.
const OPPOSITE_BISHOPS_PAWN: Score = 4;
/// The most opposite-coloured bishops may be scaled to.
const OPPOSITE_BISHOPS_CAP: Score = 48;
/// The scale of an ending before the stronger side's pawns count.
const PAWN_BASE: Score = 48;
/// What each of the stronger side's pawns adds in an ordinary ending.
const PAWN_STEP: Score = 2;

/// Returns the fraction of [`FULL`] the endgame score should count at.
///
/// The stronger side is the side the endgame score favours; an even score
/// is not scaled, because there is nothing to scale.
pub(super) fn endgame_scale(board: &Board, end_game: Score) -> Score {
    if end_game == 0 {
        return FULL;
    }
    let strong = if end_game > 0 {
        Color::White
    } else {
        Color::Black
    };
    let weak = !strong;
    let strong_pawns = board.colored_pieces(strong, Piece::Pawn).len() as Score;

    if is_dead_material(board) {
        return DEAD;
    }
    if strong_pawns == 0 {
        let advantage = non_pawn_material(board, strong) - non_pawn_material(board, weak);
        return if advantage <= piece_value(Piece::Bishop) {
            BARE_MINOR
        } else if advantage < piece_value(Piece::Rook) {
            PAWNLESS_PIECE
        } else {
            FULL
        };
    }
    if opposite_coloured_bishops_only(board) {
        return (OPPOSITE_BISHOPS + OPPOSITE_BISHOPS_PAWN * strong_pawns).min(OPPOSITE_BISHOPS_CAP);
    }
    (PAWN_BASE + PAWN_STEP * strong_pawns).min(FULL)
}

/// Sum of a side's non-pawn material at the exchange values.
fn non_pawn_material(board: &Board, color: Color) -> Score {
    [Piece::Knight, Piece::Bishop, Piece::Rook, Piece::Queen]
        .into_iter()
        .map(|piece| piece_value(piece) * board.colored_pieces(color, piece).len() as Score)
        .sum()
}

/// Neither side has enough to mate: no pawns, rooks or queens, and at most
/// one minor piece, or bishops that all stand on one colour.
///
/// This is the rule search already applies to adjudicate a draw; here it
/// also removes whatever the tables think of the pieces' placement.
pub(crate) fn is_dead_material(board: &Board) -> bool {
    if !board.pieces(Piece::Pawn).is_empty()
        || !board.pieces(Piece::Rook).is_empty()
        || !board.pieces(Piece::Queen).is_empty()
    {
        return false;
    }
    let knights = board.pieces(Piece::Knight);
    let bishops = board.pieces(Piece::Bishop);
    if knights.len() + bishops.len() <= 1 {
        return true;
    }
    knights.is_empty()
        && ((bishops & BitBoard::DARK_SQUARES) == bishops
            || (bishops & BitBoard::LIGHT_SQUARES) == bishops)
}

/// Each side has exactly one bishop, on opposite colours, and no other piece.
fn opposite_coloured_bishops_only(board: &Board) -> bool {
    if !board.pieces(Piece::Knight).is_empty()
        || !board.pieces(Piece::Rook).is_empty()
        || !board.pieces(Piece::Queen).is_empty()
    {
        return false;
    }
    let white = board.colored_pieces(Color::White, Piece::Bishop);
    let black = board.colored_pieces(Color::Black, Piece::Bishop);
    if white.len() != 1 || black.len() != 1 {
        return false;
    }
    let bishops = white | black;
    (bishops & BitBoard::DARK_SQUARES).len() == 1
}

#[cfg(test)]
mod tests {
    use super::{BARE_MINOR, DEAD, FULL, PAWNLESS_PIECE, endgame_scale};
    use cozy_chess::Board;

    fn scale(fen: &str, end_game: i32) -> i32 {
        endgame_scale(&fen.parse::<Board>().unwrap(), end_game)
    }

    #[test]
    fn dead_material_scales_to_nothing_and_a_level_score_is_not_scaled() {
        assert_eq!(scale("4k3/8/8/8/8/8/8/4KB2 w - - 0 1", 300), DEAD);
        assert_eq!(scale("4k3/8/8/8/8/8/8/4KB2 w - - 0 1", 0), FULL);
        // Two bishops on one colour cannot mate either; on both colours they can.
        assert_eq!(scale("4k3/8/8/8/8/8/8/1B1BK3 w - - 0 1", 600), DEAD);
        assert_eq!(scale("4k3/8/8/8/8/8/8/2B1KB2 w - - 0 1", 600), FULL);
    }

    #[test]
    fn a_pawnless_side_needs_more_than_a_minor_piece() {
        // A rook against a bishop is a book draw; a queen against a rook is
        // a hard win; a rook against nothing, or two bishops, is a plain one.
        assert_eq!(scale("4k3/8/8/8/8/8/8/2b1KR2 w - - 0 1", 150), BARE_MINOR);
        assert_eq!(
            scale("4k3/8/8/8/8/8/8/1r1QK3 w - - 0 1", 400),
            PAWNLESS_PIECE
        );
        assert_eq!(scale("4k3/8/8/8/8/8/8/4KR2 w - - 0 1", 500), FULL);
        assert_eq!(scale("4k3/8/8/8/8/8/8/2BBK3 w - - 0 1", 650), FULL);
        // With a pawn the ordinary rule applies and grows with the pawns.
        assert!(scale("4k3/8/8/8/8/8/4P3/2b1KR2 w - - 0 1", 300) > PAWNLESS_PIECE);
        assert!(
            scale("4k3/8/8/8/8/8/3PP3/4KR2 w - - 0 1", 600)
                > scale("4k3/8/8/8/8/8/4P3/4KR2 w - - 0 1", 600)
        );
    }

    #[test]
    fn opposite_bishops_hold_with_pawns_down() {
        // The white bishop on d1 stands on a light square; d4 is dark, c4 is
        // light.
        let opposite = scale("4k3/8/8/8/3b4/8/3PP3/3BK3 w - - 0 1", 200);
        let same = scale("4k3/8/8/8/2b5/8/3PP3/3BK3 w - - 0 1", 200);
        assert!(opposite < same);
        assert!(opposite <= FULL / 2);
        // The rule is about bishops alone; a knight beside them ends it.
        assert_eq!(
            scale("4k3/8/8/8/3b4/8/3PP3/2NBK3 w - - 0 1", 200),
            scale("4k3/8/8/8/2b5/8/3PP3/2NBK3 w - - 0 1", 200)
        );
    }

    #[test]
    fn the_stronger_side_is_the_one_the_score_favours() {
        // Black is the pawnless side here, a rook against a rook and two
        // pawns, and the rule applies only when the score says Black is ahead.
        let fen = "4k3/8/8/8/8/8/3PP3/1r2KR2 w - - 0 1";
        assert_eq!(scale(fen, -100), BARE_MINOR);
        assert!(scale(fen, 100) > PAWNLESS_PIECE);
    }
}
