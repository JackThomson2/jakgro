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
        92, 132, 56, 88, 57, 124, 20, -29, //
        -25, -9, 15, 24, 49, 40, 13, -31, //
        -25, -1, -14, 12, 7, 0, 0, -37, //
        -43, -22, -18, -2, 2, -5, -15, -42, //
        -49, -29, -21, -31, -15, -20, -10, -36, //
        -49, -16, -31, -36, -29, -2, 11, -50, //
        0, 0, 0, 0, 0, 0, 0, 0, //
    ],
    end_game: [
        0, 0, 0, 0, 0, 0, 0, 0, //
        143, 140, 131, 101, 109, 98, 135, 159, //
        56, 65, 40, 20, 7, 2, 50, 48, //
        0, -14, -23, -41, -43, -43, -29, -23, //
        -19, -36, -47, -53, -61, -47, -39, -41, //
        -36, -41, -58, -53, -64, -51, -60, -57, //
        -25, -41, -49, -28, -37, -51, -52, -40, //
        0, 0, 0, 0, 0, 0, 0, 0, //
    ],
};

static KNIGHT: Table = Table {
    middle_game: [
        -187, -99, -37, -54, 69, -108, -16, -118, //
        -80, -46, 79, 41, 26, 69, 8, -20, //
        -51, 66, 43, 75, 95, 144, 82, 49, //
        -7, 23, 26, 66, 35, 75, 23, 33, //
        -17, 5, 21, 13, 27, 30, 26, -5, //
        -34, -1, 9, 18, 20, 22, 23, -19, //
        -31, -57, -19, 0, -2, 17, -16, -20, //
        -117, -23, -63, -27, -15, -28, -18, -25, //
    ],
    end_game: [
        -45, -24, 3, -12, -15, -13, -53, -93, //
        -11, 11, -13, 14, 10, -11, -10, -39, //
        -7, -4, 33, 29, 17, 9, 0, -29, //
        -1, 22, 41, 40, 42, 26, 25, 1, //
        -5, 13, 35, 43, 30, 38, 23, 2, //
        -12, 18, 15, 29, 24, 8, -7, -5, //
        -28, -4, 5, 10, 19, -9, -9, -32, //
        -15, -35, -6, 5, -3, -2, -35, -54, //
    ],
};

static BISHOP: Table = Table {
    middle_game: [
        -39, -2, -99, -47, -34, -54, 2, -15, //
        -36, 5, -24, -20, 27, 58, 13, -59, //
        -22, 35, 38, 37, 34, 52, 38, -5, //
        -11, -2, 14, 43, 28, 33, -1, -6, //
        -12, 10, 10, 25, 28, 0, 6, -3, //
        4, 15, 14, 5, 7, 21, 9, 6, //
        -3, 4, 14, -1, 7, 17, 38, -5, //
        -41, -6, -8, -30, -19, -20, -50, -27, //
    ],
    end_game: [
        -10, -16, -6, -2, -1, -5, -15, -22, //
        -3, -5, 14, -7, 3, -9, 0, -12, //
        9, -3, 5, 6, 1, 13, 6, 8, //
        0, 13, 17, 10, 15, 12, 7, 8, //
        -1, 4, 17, 22, 9, 12, 6, -8, //
        -8, 2, 16, 11, 17, 6, -6, -13, //
        -9, -13, -5, 7, 6, -4, -15, -25, //
        -17, -5, -13, 1, -4, -9, -1, -12, //
    ],
};

static ROOK: Table = Table {
    middle_game: [
        26, 38, 26, 49, 60, 2, 27, 36, //
        22, 29, 50, 62, 83, 68, 20, 40, //
        -11, 15, 23, 32, 14, 43, 59, 10, //
        -32, -19, 1, 24, 14, 28, -16, -31, //
        -46, -40, -23, -16, -2, -17, -3, -36, //
        -58, -40, -25, -30, -8, -7, -10, -49, //
        -56, -26, -32, -17, -6, 8, -16, -92, //
        -31, -20, -20, 0, 9, -8, -40, -37, //
    ],
    end_game: [
        14, 6, 16, 13, 8, 12, 10, -3, //
        11, 13, 6, 10, -1, 6, 10, 0, //
        12, 8, 11, 5, 5, -4, -5, -3, //
        11, 9, 17, 1, 0, 1, 1, 2, //
        8, 6, 12, 4, -7, -3, -9, -14, //
        -4, -3, -5, -2, -9, -12, -8, -17, //
        -9, -7, -2, -1, -12, -13, -16, -7, //
        -6, -4, 0, -13, -16, -16, 3, -16, //
    ],
};

static QUEEN: Table = Table {
    middle_game: [
        -33, -1, 30, 8, 62, 46, 44, 44, //
        -24, -44, -8, -2, -21, 60, 29, 53, //
        -14, -23, 7, 8, 30, 57, 49, 53, //
        -34, -30, -18, -24, -5, 17, -9, -4, //
        -9, -32, -9, -12, -8, -10, -4, -6, //
        -12, 1, -14, -6, -3, 1, 14, 2, //
        -41, -10, 7, 1, 7, 16, -9, -3, //
        -8, -23, -1, 14, -23, -29, -38, -60, //
    ],
    end_game: [
        -19, 15, 15, 19, 20, 10, 1, 11, //
        -27, 14, 25, 36, 55, 18, 23, -11, //
        -30, -4, 1, 44, 43, 28, 13, 0, //
        -8, 15, 18, 40, 54, 36, 53, 30, //
        -28, 21, 14, 42, 24, 29, 32, 17, //
        -26, -37, 7, -6, 3, 11, 2, -6, //
        -35, -36, -45, -27, -26, -36, -52, -47, //
        -48, -41, -32, -55, -19, -47, -33, -56, //
    ],
};

static KING: Table = Table {
    middle_game: [
        -55, 43, 35, 1, -46, -20, 20, 32, //
        50, 16, -5, 10, 9, 13, -25, -15, //
        7, 45, 20, -1, -5, 23, 41, -7, //
        -2, -5, 4, -13, -17, -11, 1, -23, //
        -37, 15, -13, -27, -36, -33, -20, -41, //
        1, 2, -11, -34, -33, -19, -1, -12, //
        20, 26, 6, -45, -27, 6, 24, 27, //
        3, 49, 35, -34, 19, -9, 45, 35, //
    ],
    end_game: [
        -86, -42, -23, -23, -15, 14, 1, -22, //
        -17, 16, 16, 16, 15, 39, 23, 8, //
        8, 18, 25, 16, 23, 42, 44, 12, //
        -13, 22, 25, 25, 27, 32, 26, 4, //
        -23, -7, 20, 25, 22, 20, 8, -16, //
        -24, -2, 4, 21, 26, 9, -1, -16, //
        -33, -17, -1, 16, 14, 4, -10, -32, //
        -58, -44, -21, -8, -31, -7, -37, -58, //
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
