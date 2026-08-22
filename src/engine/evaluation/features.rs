use cozy_chess::{
    Board, Color, Piece, Square, get_bishop_moves, get_king_moves, get_knight_moves,
    get_pawn_attacks, get_rook_moves,
};

use super::EvalFeatures;

pub(super) fn extract(board: &Board) -> EvalFeatures {
    let mut features = EvalFeatures::default();

    for color in [Color::White, Color::Black] {
        let sign = if color == Color::White { 1 } else { -1 };
        features.pawns += sign * board.colored_pieces(color, Piece::Pawn).len() as i32;
        features.knights += sign * board.colored_pieces(color, Piece::Knight).len() as i32;
        features.bishops += sign * board.colored_pieces(color, Piece::Bishop).len() as i32;
        features.rooks += sign * board.colored_pieces(color, Piece::Rook).len() as i32;
        features.queens += sign * board.colored_pieces(color, Piece::Queen).len() as i32;

        let bishops = board.colored_pieces(color, Piece::Bishop).len();
        features.bishop_pair += sign * i32::from(bishops >= 2);
        features.activity += sign * activity(board, color);
        features.mobility += sign * mobility(board, color);

        let pawns = pawn_features(board, color);
        features.doubled_pawns += sign * pawns.doubled;
        features.isolated_pawns += sign * pawns.isolated;
        features.passed_pawns += sign * pawns.passed;

        let (shelter, open_files) = king_safety(board, color);
        features.king_shelter += sign * shelter;
        features.open_king_files += sign * open_files;
    }

    features
}

pub(super) fn phase(board: &Board) -> i32 {
    let queens = board.pieces(Piece::Queen).len() as i32;
    let rooks = board.pieces(Piece::Rook).len() as i32;
    let bishops = board.pieces(Piece::Bishop).len() as i32;
    let knights = board.pieces(Piece::Knight).len() as i32;
    (queens * 4 + rooks * 2 + bishops + knights).min(24)
}

fn activity(board: &Board, color: Color) -> i32 {
    let mut score = 0;
    for piece in [Piece::Knight, Piece::Bishop, Piece::Rook, Piece::Queen] {
        for square in board.colored_pieces(color, piece) {
            score += centrality(square);
        }
    }
    for square in board.colored_pieces(color, Piece::Pawn) {
        let rank = square.rank() as i32;
        score += if color == Color::White {
            (rank - 1).max(0)
        } else {
            (6 - rank).max(0)
        };
    }
    score
}

fn centrality(square: Square) -> i32 {
    let file = square.file() as i32;
    let rank = square.rank() as i32;
    let file_distance = (file - 3).abs().min((file - 4).abs());
    let rank_distance = (rank - 3).abs().min((rank - 4).abs());
    6 - file_distance - rank_distance
}

fn mobility(board: &Board, color: Color) -> i32 {
    let occupied = board.occupied();
    let friendly = board.colors(color);
    let mut total = 0;

    for piece in [
        Piece::Pawn,
        Piece::Knight,
        Piece::Bishop,
        Piece::Rook,
        Piece::Queen,
        Piece::King,
    ] {
        for square in board.colored_pieces(color, piece) {
            let attacks = match piece {
                Piece::Pawn => get_pawn_attacks(square, color),
                Piece::Knight => get_knight_moves(square),
                Piece::Bishop => get_bishop_moves(square, occupied),
                Piece::Rook => get_rook_moves(square, occupied),
                Piece::Queen => {
                    get_bishop_moves(square, occupied) | get_rook_moves(square, occupied)
                }
                Piece::King => get_king_moves(square),
            };
            total += (attacks & !friendly).len() as i32;
        }
    }

    total
}

#[derive(Clone, Copy, Debug, Default)]
struct PawnFeatures {
    doubled: i32,
    isolated: i32,
    passed: i32,
}

fn pawn_features(board: &Board, color: Color) -> PawnFeatures {
    let pawns = board.colored_pieces(color, Piece::Pawn);
    let enemy_pawns = board.colored_pieces(!color, Piece::Pawn);
    let mut files = [0_u8; 8];
    for square in pawns {
        files[square.file() as usize] += 1;
    }

    let mut result = PawnFeatures::default();
    for (file, &count) in files.iter().enumerate() {
        result.doubled += i32::from(count.saturating_sub(1));
        if count > 0 && (file == 0 || files[file - 1] == 0) && (file == 7 || files[file + 1] == 0) {
            result.isolated += i32::from(count);
        }
    }

    for square in pawns {
        let file = square.file() as i32;
        let rank = square.rank() as i32;
        let blocked = enemy_pawns.into_iter().any(|enemy| {
            let enemy_file = enemy.file() as i32;
            let enemy_rank = enemy.rank() as i32;
            (enemy_file - file).abs() <= 1
                && if color == Color::White {
                    enemy_rank > rank
                } else {
                    enemy_rank < rank
                }
        });
        if !blocked {
            let advance = if color == Color::White {
                rank
            } else {
                7 - rank
            };
            result.passed += advance.max(1);
        }
    }

    result
}

fn king_safety(board: &Board, color: Color) -> (i32, i32) {
    let king = board.king(color);
    let king_file = king.file() as i32;
    let king_rank = king.rank() as i32;
    let pawns = board.colored_pieces(color, Piece::Pawn);
    let mut shelter = 0;

    for pawn in pawns {
        let file_delta = (pawn.file() as i32 - king_file).abs();
        let rank_delta = if color == Color::White {
            pawn.rank() as i32 - king_rank
        } else {
            king_rank - pawn.rank() as i32
        };
        if file_delta <= 1 && (1..=2).contains(&rank_delta) {
            shelter += 1;
        }
    }

    let mut open_files = 0;
    for file in (king_file - 1).max(0)..=(king_file + 1).min(7) {
        if !pawns.into_iter().any(|pawn| pawn.file() as i32 == file) {
            open_files += 1;
        }
    }

    (shelter, open_files)
}
