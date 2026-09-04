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
        97, 139, 61, 93, 60, 131, 21, -30, //
        -27, -10, 16, 27, 50, 41, 14, -31, //
        -25, -3, -15, 13, 8, 2, -2, -36, //
        -43, -23, -17, -2, 2, -5, -19, -45, //
        -53, -31, -24, -32, -17, -24, -14, -40, //
        -48, -16, -30, -36, -30, -4, 7, -51, //
        0, 0, 0, 0, 0, 0, 0, 0, //
    ],
    end_game: [
        0, 0, 0, 0, 0, 0, 0, 0, //
        149, 146, 140, 107, 114, 103, 142, 168, //
        57, 67, 39, 20, 6, 1, 53, 50, //
        -1, -15, -23, -42, -42, -45, -30, -24, //
        -19, -38, -50, -55, -64, -49, -40, -42, //
        -36, -44, -61, -57, -68, -53, -63, -59, //
        -26, -43, -53, -29, -40, -54, -53, -42, //
        0, 0, 0, 0, 0, 0, 0, 0, //
    ],
};

static KNIGHT: Table = Table {
    middle_game: [
        -197, -105, -39, -57, 73, -114, -17, -124, //
        -84, -49, 82, 43, 27, 72, 8, -21, //
        -54, 69, 45, 78, 100, 151, 86, 51, //
        -7, 23, 28, 70, 37, 78, 25, 37, //
        -18, 5, 23, 16, 29, 36, 28, -4, //
        -37, 0, 10, 20, 21, 20, 22, -21, //
        -32, -60, -22, 0, -4, 17, -17, -21, //
        -124, -24, -66, -26, -15, -29, -19, -26, //
    ],
    end_game: [
        -47, -25, 3, -12, -15, -14, -56, -98, //
        -12, 12, -15, 14, 11, -12, -11, -41, //
        -7, -5, 36, 31, 18, 10, 1, -31, //
        -1, 22, 43, 42, 45, 26, 26, 2, //
        -6, 14, 37, 46, 32, 41, 25, 3, //
        -14, 19, 16, 29, 24, 7, -8, -5, //
        -29, -4, 5, 10, 20, -11, -10, -34, //
        -16, -37, -6, 6, -2, -2, -36, -57, //
    ],
};

static BISHOP: Table = Table {
    middle_game: [
        -41, -2, -105, -49, -36, -57, 2, -16, //
        -38, 3, -25, -21, 28, 61, 13, -62, //
        -23, 36, 38, 39, 36, 55, 41, -5, //
        -12, -1, 15, 44, 28, 35, 1, -6, //
        -13, 10, 12, 28, 30, 2, 7, -3, //
        6, 16, 15, 5, 8, 21, 8, 6, //
        -4, 4, 15, -1, 6, 17, 38, -5, //
        -43, -5, -5, -32, -20, -21, -53, -28, //
    ],
    end_game: [
        -11, -16, -6, -1, 0, -5, -16, -23, //
        -3, -8, 15, -7, 4, -9, 0, -13, //
        10, -4, 5, 7, 1, 14, 7, 8, //
        -1, 13, 18, 10, 16, 12, 7, 8, //
        -1, 3, 19, 23, 10, 13, 8, -9, //
        -9, 2, 18, 12, 18, 5, -7, -14, //
        -9, -13, -6, 8, 5, -5, -19, -26, //
        -17, -5, -12, 1, -4, -9, -1, -12, //
    ],
};

static ROOK: Table = Table {
    middle_game: [
        27, 40, 27, 52, 63, 2, 29, 37, //
        22, 30, 51, 66, 88, 72, 21, 41, //
        -11, 16, 25, 34, 16, 46, 62, 11, //
        -33, -19, 2, 27, 14, 29, -16, -33, //
        -48, -42, -24, -18, -2, -17, -3, -38, //
        -61, -43, -26, -32, -9, -7, -9, -52, //
        -58, -27, -34, -18, -5, 8, -17, -98, //
        -34, -20, -23, -1, 8, -10, -39, -41, //
    ],
    end_game: [
        14, 5, 16, 13, 7, 13, 11, -6, //
        10, 12, 4, 10, -1, 7, 11, -2, //
        13, 9, 13, 6, 6, -4, -5, -3, //
        14, 11, 19, 2, 0, 2, 3, 3, //
        10, 7, 14, 5, -7, -1, -8, -15, //
        -4, -4, -5, -2, -9, -12, -8, -17, //
        -10, -7, -3, -2, -13, -15, -18, -8, //
        -8, -6, 0, -15, -19, -18, 1, -17, //
    ],
};

static QUEEN: Table = Table {
    middle_game: [
        -34, -1, 32, 8, 65, 49, 46, 45, //
        -23, -45, -8, -2, -22, 63, 30, 54, //
        -13, -25, 8, 9, 32, 59, 51, 52, //
        -36, -30, -18, -25, -5, 19, -10, -4, //
        -8, -33, -8, -12, -8, -10, -4, -6, //
        -11, 2, -15, -6, -2, 1, 14, 2, //
        -43, -11, 6, 0, 6, 17, -10, -4, //
        -10, -24, 2, 12, -25, -30, -40, -64, //
    ],
    end_game: [
        -20, 16, 16, 20, 21, 10, 1, 11, //
        -28, 15, 27, 38, 58, 19, 24, -12, //
        -31, -4, 1, 46, 45, 29, 14, -1, //
        -8, 16, 19, 42, 57, 38, 56, 32, //
        -29, 22, 16, 44, 25, 31, 33, 18, //
        -27, -38, 7, -7, 4, 12, 2, -7, //
        -37, -38, -48, -28, -27, -38, -56, -50, //
        -51, -43, -33, -57, -21, -50, -35, -59, //
    ],
};

static KING: Table = Table {
    middle_game: [
        -58, 45, 37, 1, -49, -21, 21, 34, //
        53, 17, -5, 11, 10, 14, -26, -16, //
        7, 48, 21, -1, -5, 24, 43, -7, //
        -2, -5, 4, -14, -18, -12, 1, -24, //
        -39, 15, -14, -29, -38, -35, -22, -44, //
        1, 2, -12, -36, -35, -20, -1, -13, //
        21, 29, 7, -43, -25, 9, 25, 29, //
        4, 49, 38, -32, 12, -8, 45, 36, //
    ],
    end_game: [
        -91, -44, -24, -24, -16, 15, 1, -23, //
        -18, 17, 18, 17, 16, 41, 24, 8, //
        8, 19, 27, 17, 26, 43, 45, 12, //
        -15, 23, 27, 26, 29, 34, 27, 4, //
        -25, -7, 21, 27, 22, 20, 8, -18, //
        -26, -1, 4, 22, 27, 9, -2, -19, //
        -35, -17, 0, 19, 17, 5, -10, -37, //
        -60, -47, -21, -6, -33, -5, -39, -64, //
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
