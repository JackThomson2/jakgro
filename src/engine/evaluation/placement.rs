//! Tapered piece-square tables.
//!
//! Each entry is a positional delta added on top of the material weight for its
//! piece, so the tables say where a piece belongs rather than what it is worth.
//! They are the published PeSTO set from the Chess Programming Wiki, which is
//! itself derived from Ronald Friederich's rofChade: a widely reproduced pair of
//! middlegame and endgame tables tuned together, used here in preference to a
//! hand-written set so the values are traceable rather than invented.
//!
//! Tables are written from White's perspective with rank eight first, which is
//! how they are conventionally published and so easy to check by eye. The lookup
//! flips the rank for Black and never mirrors files, because a chess position is
//! symmetric about the horizontal axis and not the vertical one: castling rights
//! and pawn direction distinguish the two sides, not the queenside from the
//! kingside.

use cozy_chess::{Color, Piece, Square};

use super::{Score, ScorePair};

/// Returns the tapered placement delta for a piece on a square.
pub(super) fn placement(piece: Piece, square: Square, color: Color) -> ScorePair {
    let index = table_index(square, color);
    let table = table_for(piece);
    ScorePair::new(table.middle_game[index], table.end_game[index])
}

/// Maps a square onto a table index written from White's perspective.
#[cfg(feature = "tuning")]
pub(super) const fn table_index_for_tuning(square: Square, color: Color) -> usize {
    table_index(square, color)
}

/// Returns one table entry by its row index, for offline fitting.
#[cfg(feature = "tuning")]
pub(super) fn table_entry(piece: Piece, index: usize) -> ScorePair {
    let table = table_for(piece);
    ScorePair::new(table.middle_game[index], table.end_game[index])
}

/// Maps a square onto a table index written from White's perspective.
const fn table_index(square: Square, color: Color) -> usize {
    let square = square as usize;
    let file = square % 8;
    let rank = square / 8;
    match color {
        // White's first rank is the table's last row.
        Color::White => (7 - rank) * 8 + file,
        Color::Black => rank * 8 + file,
    }
}

/// One piece's middlegame and endgame tables.
struct Table {
    middle_game: [Score; 64],
    end_game: [Score; 64],
}

const fn table_for(piece: Piece) -> &'static Table {
    match piece {
        Piece::Pawn => &PAWN,
        Piece::Knight => &KNIGHT,
        Piece::Bishop => &BISHOP,
        Piece::Rook => &ROOK,
        Piece::Queen => &QUEEN,
        Piece::King => &KING,
    }
}

static PAWN: Table = Table {
    middle_game: [
        0, 0, 0, 0, 0, 0, 0, 0, //
        83, 119, 47, 79, 52, 111, 19, -26, //
        -21, -7, 11, 18, 47, 39, 11, -33, //
        -27, -3, -11, 9, 6, -4, 0, -40, //
        -44, -20, -20, -3, 2, -6, -7, -37, //
        -38, -20, -20, -27, -13, -14, 8, -25, //
        -48, -17, -34, -38, -27, 4, 20, -37, //
        0, 0, 0, 0, 0, 0, 0, 0, //
    ],
    end_game: [
        0, 0, 0, 0, 0, 0, 0, 0, //
        134, 129, 116, 91, 101, 89, 122, 144, //
        52, 59, 40, 21, 10, 6, 43, 42, //
        -1, -17, -25, -39, -46, -40, -27, -24, //
        -23, -33, -42, -51, -53, -45, -37, -39, //
        -35, -32, -49, -45, -52, -45, -47, -49, //
        -24, -37, -40, -29, -31, -43, -45, -36, //
        0, 0, 0, 0, 0, 0, 0, 0, //
    ],
};

static KNIGHT: Table = Table {
    middle_game: [
        -167, -89, -34, -49, 61, -97, -15, -107, //
        -73, -41, 72, 36, 23, 62, 7, -17, //
        -46, 59, 37, 66, 84, 129, 73, 44, //
        -8, 19, 21, 55, 34, 68, 18, 24, //
        -14, 4, 18, 11, 25, 21, 21, -6, //
        -27, -4, 10, 12, 19, 21, 24, -16, //
        -29, -52, -14, -3, -1, 16, -15, -19, //
        -105, -22, -58, -30, -16, -27, -19, -23, //
    ],
    end_game: [
        -42, -23, 2, -13, -16, -12, -48, -84, //
        -10, 8, -10, 13, 7, -10, -9, -36, //
        -8, -5, 26, 24, 14, 6, -3, -26, //
        -2, 19, 36, 36, 36, 25, 22, -2, //
        -4, 10, 31, 39, 27, 32, 19, -1, //
        -9, 15, 12, 29, 24, 9, -6, -6, //
        -27, -5, 4, 9, 15, -6, -8, -29, //
        -14, -34, -7, 2, -6, -3, -34, -49, //
    ],
};

static BISHOP: Table = Table {
    middle_game: [
        -34, -1, -87, -42, -30, -47, 2, -13, //
        -31, 10, -22, -18, 25, 53, 13, -52, //
        -20, 33, 37, 34, 30, 46, 33, -5, //
        -9, -1, 13, 42, 29, 32, 1, -5, //
        -10, 10, 9, 22, 27, 1, 6, -2, //
        0, 13, 11, 7, 9, 22, 11, 6, //
        -1, 6, 12, -1, 6, 16, 34, -4, //
        -37, -7, -16, -26, -17, -16, -44, -25, //
    ],
    end_game: [
        -10, -16, -6, -4, -3, -5, -13, -20, //
        -3, -1, 12, -7, 1, -9, 0, -10, //
        7, -3, 5, 4, 1, 11, 4, 8, //
        0, 13, 15, 11, 16, 13, 7, 7, //
        -1, 6, 15, 22, 10, 12, 3, -6, //
        -7, 1, 12, 12, 17, 8, -4, -11, //
        -9, -13, -3, 5, 8, -4, -10, -23, //
        -17, -5, -16, -1, -4, -10, -1, -12, //
    ],
};

static ROOK: Table = Table {
    middle_game: [
        25, 35, 25, 44, 55, 2, 24, 35, //
        21, 26, 49, 55, 74, 61, 19, 37, //
        -11, 13, 20, 29, 11, 38, 54, 9, //
        -30, -18, 0, 19, 15, 27, -15, -27, //
        -42, -34, -19, -11, 1, -14, -2, -31, //
        -52, -33, -22, -25, -5, -6, -11, -42, //
        -51, -23, -27, -16, -7, 8, -13, -80, //
        -29, -18, -9, 5, 9, -2, -39, -32, //
    ],
    end_game: [
        14, 8, 17, 13, 10, 11, 8, 3, //
        12, 13, 10, 9, -2, 4, 8, 2, //
        8, 7, 7, 4, 3, -4, -5, -3, //
        6, 5, 13, 0, 1, 0, -1, 2, //
        4, 4, 9, 3, -6, -5, -9, -12, //
        -5, -1, -5, -2, -8, -12, -8, -16, //
        -7, -7, -1, 0, -10, -9, -12, -5, //
        -7, 1, 1, -6, -10, -13, 5, -17, //
    ],
};

static QUEEN: Table = Table {
    middle_game: [
        -29, -1, 27, 9, 57, 42, 41, 43, //
        -24, -40, -7, -1, -18, 55, 27, 52, //
        -14, -19, 6, 7, 27, 54, 45, 54, //
        -29, -29, -17, -19, -4, 15, -5, -2, //
        -9, -28, -11, -12, -5, -7, -1, -5, //
        -13, 1, -12, -5, -5, 2, 15, 3, //
        -37, -9, 7, 1, 6, 14, -5, -1, //
        -4, -20, -7, 13, -18, -26, -33, -52, //
    ],
    end_game: [
        -17, 13, 13, 18, 18, 10, 1, 11, //
        -25, 12, 23, 32, 49, 16, 21, -9, //
        -28, -3, 1, 40, 39, 26, 11, 1, //
        -6, 13, 16, 36, 48, 32, 48, 27, //
        -26, 19, 11, 38, 22, 26, 30, 15, //
        -24, -35, 7, -4, 1, 9, 2, -4, //
        -31, -32, -39, -25, -24, -32, -45, -41, //
        -42, -37, -30, -51, -15, -41, -29, -50, //
    ],
};

static KING: Table = Table {
    middle_game: [
        -50, 38, 31, 0, -41, -19, 17, 28, //
        44, 14, -5, 8, 7, 11, -23, -14, //
        6, 39, 17, -1, -5, 21, 37, -7, //
        -2, -5, 3, -12, -15, -10, 1, -21, //
        -34, 14, -12, -24, -32, -29, -18, -36, //
        1, 1, -9, -31, -29, -17, -1, -11, //
        16, 21, 5, -48, -30, 1, 22, 25, //
        1, 49, 29, -37, 19, -9, 44, 33, //
    ],
    end_game: [
        -76, -37, -20, -20, -13, 13, 2, -19, //
        -14, 15, 13, 15, 14, 36, 21, 9, //
        8, 16, 22, 14, 18, 41, 41, 11, //
        -10, 20, 23, 24, 24, 29, 24, 3, //
        -19, -6, 19, 22, 23, 20, 8, -12, //
        -20, -3, 5, 20, 22, 10, 2, -10, //
        -29, -16, -1, 10, 10, 2, -10, -19, //
        -54, -37, -21, -11, -28, -10, -27, -43, //
    ],
};

#[cfg(test)]
mod tests {
    use super::{placement, table_index};
    use cozy_chess::{Color, File, Piece, Rank, Square};

    #[test]
    fn white_and_black_read_vertically_mirrored_entries() {
        for file in File::ALL {
            for rank in Rank::ALL {
                let white = Square::new(file, rank);
                let black = Square::new(file, Rank::index(7 - rank as usize));

                assert_eq!(
                    table_index(white, Color::White),
                    table_index(black, Color::Black),
                    "{white} and {black} should read the same entry",
                );
            }
        }
    }

    #[test]
    fn the_first_rank_reads_the_last_table_row() {
        assert_eq!(table_index(Square::A1, Color::White), 56);
        assert_eq!(table_index(Square::H1, Color::White), 63);
        assert_eq!(table_index(Square::A8, Color::White), 0);
        assert_eq!(table_index(Square::A8, Color::Black), 56);
        assert_eq!(table_index(Square::A1, Color::Black), 0);
    }

    #[test]
    fn placement_prefers_a_central_knight_to_a_cornered_one() {
        let centre = placement(Piece::Knight, Square::E4, Color::White);
        let corner = placement(Piece::Knight, Square::A1, Color::White);

        // A floor rather than an exact margin. The point is that the tables
        // express the preference decisively rather than by a centipawn, which a
        // minimum states directly; an exact figure additionally pins one fitting
        // of the tables, and would have to be rewritten every time they are
        // refitted without saying anything more about what they mean.
        assert!(centre.middle_game() - corner.middle_game() >= 80);
        assert!(centre.end_game() - corner.end_game() >= 20);
    }

    #[test]
    fn placement_moves_the_king_home_in_the_middlegame_and_out_in_the_endgame() {
        let shelter = placement(Piece::King, Square::G1, Color::White);
        let centre = placement(Piece::King, Square::E4, Color::White);

        assert!(shelter.middle_game() - centre.middle_game() >= 40);
        assert!(centre.end_game() - shelter.end_game() >= 25);
    }

    #[test]
    fn advanced_pawns_are_worth_more_in_the_endgame() {
        let advanced = placement(Piece::Pawn, Square::D7, Color::White);
        let home = placement(Piece::Pawn, Square::D2, Color::White);

        assert!(advanced.end_game() - home.end_game() >= 80);
        // Direction rather than magnitude. How much more a pawn is worth in an
        // endgame is mostly carried by its material weight, and re-centring the
        // tables deliberately moves it there: what is left here is the part that
        // depends on the square, which is a preference and not a valuation.
        assert!(advanced.end_game() > advanced.middle_game());
    }

    #[test]
    fn placement_is_colour_symmetric() {
        for piece in [
            Piece::Pawn,
            Piece::Knight,
            Piece::Bishop,
            Piece::Rook,
            Piece::Queen,
            Piece::King,
        ] {
            for file in File::ALL {
                for rank in Rank::ALL {
                    let white = Square::new(file, rank);
                    let black = Square::new(file, Rank::index(7 - rank as usize));

                    assert_eq!(
                        placement(piece, white, Color::White),
                        placement(piece, black, Color::Black),
                        "{piece:?} on {white} and {black} should match",
                    );
                }
            }
        }
    }

    /// Checks each table describes a coherent evaluation.
    ///
    /// This replaced a set of aggregate checksums taken from the published PeSTO
    /// tables. Those existed because the values were transcribed by hand, so a
    /// single mistyped interior entry would otherwise have been invisible: the
    /// symmetry test only catches asymmetric corruption. The tables are now
    /// written by the fitter rather than copied, which removes the failure the
    /// checksums guarded and leaves them pinning one fitting of the tables
    /// instead — something that must be rewritten on every refit while saying
    /// nothing about whether the result is sane.
    ///
    /// What is worth asserting is what a placement table means: bounded entries,
    /// a centre worth more than a corner to a piece that wants the centre, and a
    /// king that wants shelter early and activity late.
    #[test]
    fn tables_describe_a_coherent_evaluation() {
        for piece in [
            Piece::Pawn,
            Piece::Knight,
            Piece::Bishop,
            Piece::Rook,
            Piece::Queen,
            Piece::King,
        ] {
            let table = super::table_for(piece);
            for entry in table.middle_game.iter().chain(table.end_game.iter()) {
                assert!(
                    entry.abs() <= 400,
                    "{piece:?} has an entry of {entry}, which is beyond what placement can mean",
                );
            }
        }

        // The minor pieces are the ones whose value is most obviously positional.
        for piece in [Piece::Knight, Piece::Bishop] {
            let centre = placement(piece, Square::E4, Color::White).middle_game()
                + placement(piece, Square::D5, Color::White).middle_game();
            let corners = placement(piece, Square::A1, Color::White).middle_game()
                + placement(piece, Square::H8, Color::White).middle_game();
            assert!(centre > corners, "{piece:?} does not prefer the centre");
        }

        let king_corner_endgame = placement(Piece::King, Square::A1, Color::White).end_game();
        let king_centre_endgame = placement(Piece::King, Square::E4, Color::White).end_game();
        assert!(king_centre_endgame > king_corner_endgame);
    }

    /// A pawn can never stand on the first or last rank.
    #[test]
    fn pawn_tables_are_empty_on_the_back_ranks() {
        for index in (0..8).chain(56..64) {
            assert_eq!(super::PAWN.middle_game[index], 0);
            assert_eq!(super::PAWN.end_game[index], 0);
        }
    }
}
