use cozy_chess::{
    Board, Color, Piece, Square, get_bishop_moves, get_king_moves, get_knight_moves,
    get_pawn_attacks, get_rook_moves,
};

use super::{AttackProfile, EvalFeatures, piece_value};

pub(super) fn extract(board: &Board) -> EvalFeatures {
    let mut features = EvalFeatures::default();
    let attacks = attacking_features(board);
    let white_attack = attacks[Color::White as usize];
    let black_attack = attacks[Color::Black as usize];
    features.white_attack = white_attack;
    features.black_attack = black_attack;

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
        let attack = if color == Color::White {
            white_attack
        } else {
            black_attack
        };
        features.king_pressure += sign * attack.king_pressure;
        features.pawn_storm += sign * attack.pawn_storm;
        features.threats += sign * attack.threats;
        features.space += sign * attack.space;
        features.coordination += sign * attack.coordination();
        features.supported_threats += sign * attack.supported_threats;
        features.open_lines += sign * attack.open_lines;
        features.pawn_breaks += sign * attack.pawn_breaks;

        let pawns = pawn_features(board, color);
        features.doubled_pawns += sign * pawns.doubled;
        features.isolated_pawns += sign * pawns.isolated;
        features.passed_pawns += sign * pawns.passed;

        let (shelter, open_files) = king_safety(board, color);
        features.king_shelter += sign * shelter;
        features.open_king_files += sign * open_files;
    }

    features.initiative = if board.side_to_move() == Color::White {
        1
    } else {
        -1
    };
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

fn attacking_features(board: &Board) -> [AttackProfile; 2] {
    let occupied = board.occupied();
    let king_zones = [
        get_king_moves(board.king(Color::White)) | board.colored_pieces(Color::White, Piece::King),
        get_king_moves(board.king(Color::Black)) | board.colored_pieces(Color::Black, Piece::King),
    ];
    let mut profiles = [AttackProfile::default(); 2];
    let mut attack_counts = [[0_u8; 64]; 2];
    let mut zone_defenders = [0_i32; 2];

    for color in [Color::White, Color::Black] {
        let index = color as usize;
        let enemy = !color;
        let enemy_king = board.king(enemy);
        let enemy_king_zone = king_zones[enemy as usize];
        let enemy_pieces = board.colors(enemy);
        let friendly_pieces = board.colors(color);
        let mut result = AttackProfile::default();
        let mut attacker_mask = 0_u8;

        for piece in [
            Piece::Pawn,
            Piece::Knight,
            Piece::Bishop,
            Piece::Rook,
            Piece::Queen,
            Piece::King,
        ] {
            for square in board.colored_pieces(color, piece) {
                let raw_attacks = attacks_from(piece, square, color, occupied);
                let attacks = raw_attacks & !friendly_pieces;
                if piece != Piece::King {
                    for target in raw_attacks {
                        attack_counts[index][target as usize] += 1;
                    }
                    zone_defenders[index] +=
                        i32::from(!(raw_attacks & king_zones[index]).is_empty());
                }

                let zone_hits = (attacks & enemy_king_zone).len() as i32;
                if zone_hits > 0 && piece != Piece::King {
                    result.attackers += 1;
                    attacker_mask |= 1 << piece_index(piece);
                    let weight = match piece {
                        Piece::Pawn => 3,
                        Piece::Knight | Piece::Bishop => 4,
                        Piece::Rook => 3,
                        Piece::Queen => 2,
                        Piece::King => 0,
                    };
                    result.king_pressure += zone_hits * weight;
                    if matches!(piece, Piece::Bishop | Piece::Rook | Piece::Queen) {
                        result.open_lines += 1;
                    }
                }

                for target in attacks & enemy_pieces {
                    let Some(target_piece) = board.piece_on(target) else {
                        continue;
                    };
                    if piece != Piece::King
                        && target_piece != Piece::King
                        && piece_value(piece) < piece_value(target_piece)
                    {
                        result.threats +=
                            1 + (piece_value(target_piece) - piece_value(piece)) / 100;
                    }
                }

                result.space += attacks
                    .into_iter()
                    .filter(|target| {
                        let rank = target.rank() as i32;
                        if color == Color::White {
                            rank >= 4
                        } else {
                            rank <= 3
                        }
                    })
                    .count() as i32;
            }
        }

        result.attacker_variety = attacker_mask.count_ones() as i32;
        result.king_pressure += result.attackers * result.attackers * 2;
        let king_file = enemy_king.file() as i32;
        let king_rank = enemy_king.rank() as i32;
        let enemy_pawns = board.colored_pieces(enemy, Piece::Pawn);
        for pawn in board.colored_pieces(color, Piece::Pawn) {
            if (pawn.file() as i32 - king_file).abs() <= 1 {
                let distance = if color == Color::White {
                    king_rank - pawn.rank() as i32
                } else {
                    pawn.rank() as i32 - king_rank
                };
                if (1..=4).contains(&distance) {
                    result.pawn_storm += 5 - distance;
                }
            }
            result.pawn_breaks += (get_pawn_attacks(pawn, color) & enemy_pawns)
                .into_iter()
                .filter(|target| (target.file() as i32 - king_file).abs() <= 1)
                .count() as i32;
        }
        profiles[index] = result;
    }

    for color in [Color::White, Color::Black] {
        let index = color as usize;
        let enemy = !color;
        let result = &mut profiles[index];
        result.defender_shortage = (result.attackers - zone_defenders[enemy as usize]).max(0);
        for target in board.colors(enemy) {
            let Some(target_piece) = board.piece_on(target) else {
                continue;
            };
            if target_piece == Piece::King {
                continue;
            }
            let attackers = i32::from(attack_counts[index][target as usize]);
            if attackers >= 2 {
                result.supported_threats += (attackers - 1) * (1 + piece_value(target_piece) / 300);
            }
        }
    }

    profiles
}

#[cfg(test)]
fn reference_attacking_features(board: &Board, color: Color) -> AttackProfile {
    let occupied = board.occupied();
    let enemy = !color;
    let enemy_king = board.king(enemy);
    let king_zone = get_king_moves(enemy_king) | board.colored_pieces(enemy, Piece::King);
    let enemy_pieces = board.colors(enemy);
    let mut result = AttackProfile::default();
    let mut attacker_mask = 0_u8;

    for piece in [
        Piece::Pawn,
        Piece::Knight,
        Piece::Bishop,
        Piece::Rook,
        Piece::Queen,
        Piece::King,
    ] {
        for square in board.colored_pieces(color, piece) {
            let attacks = attacks_from(piece, square, color, occupied) & !board.colors(color);
            let zone_hits = (attacks & king_zone).len() as i32;
            if zone_hits > 0 && piece != Piece::King {
                result.attackers += 1;
                attacker_mask |= 1 << piece_index(piece);
                let weight = match piece {
                    Piece::Pawn => 3,
                    Piece::Knight | Piece::Bishop => 4,
                    Piece::Rook => 3,
                    Piece::Queen => 2,
                    Piece::King => 0,
                };
                result.king_pressure += zone_hits * weight;
                if matches!(piece, Piece::Bishop | Piece::Rook | Piece::Queen) {
                    result.open_lines += 1;
                }
            }

            for target in attacks & enemy_pieces {
                let Some(target_piece) = board.piece_on(target) else {
                    continue;
                };
                if piece != Piece::King
                    && target_piece != Piece::King
                    && piece_value(piece) < piece_value(target_piece)
                {
                    result.threats += 1 + (piece_value(target_piece) - piece_value(piece)) / 100;
                }
            }

            result.space += attacks
                .into_iter()
                .filter(|target| {
                    let rank = target.rank() as i32;
                    if color == Color::White {
                        rank >= 4
                    } else {
                        rank <= 3
                    }
                })
                .count() as i32;
        }
    }

    result.attacker_variety = attacker_mask.count_ones() as i32;
    let defenders = zone_defenders(board, enemy, king_zone, occupied);
    result.defender_shortage = (result.attackers - defenders).max(0);
    result.king_pressure += result.attackers * result.attackers * 2;
    for target in enemy_pieces {
        let Some(target_piece) = board.piece_on(target) else {
            continue;
        };
        if target_piece == Piece::King {
            continue;
        }
        let attackers = attackers_to(board, color, target, occupied);
        if attackers >= 2 {
            result.supported_threats += (attackers - 1) * (1 + piece_value(target_piece) / 300);
        }
    }

    let king_file = enemy_king.file() as i32;
    let king_rank = enemy_king.rank() as i32;
    let enemy_pawns = board.colored_pieces(enemy, Piece::Pawn);
    for pawn in board.colored_pieces(color, Piece::Pawn) {
        if (pawn.file() as i32 - king_file).abs() <= 1 {
            let distance = if color == Color::White {
                king_rank - pawn.rank() as i32
            } else {
                pawn.rank() as i32 - king_rank
            };
            if (1..=4).contains(&distance) {
                result.pawn_storm += 5 - distance;
            }
        }
        result.pawn_breaks += (get_pawn_attacks(pawn, color) & enemy_pawns)
            .into_iter()
            .filter(|target| (target.file() as i32 - king_file).abs() <= 1)
            .count() as i32;
    }

    result
}

fn piece_index(piece: Piece) -> u8 {
    match piece {
        Piece::Pawn => 0,
        Piece::Knight => 1,
        Piece::Bishop => 2,
        Piece::Rook => 3,
        Piece::Queen => 4,
        Piece::King => 5,
    }
}

#[cfg(test)]
fn attackers_to(
    board: &Board,
    color: Color,
    target: Square,
    occupied: cozy_chess::BitBoard,
) -> i32 {
    let mut attackers = 0;
    for piece in [
        Piece::Pawn,
        Piece::Knight,
        Piece::Bishop,
        Piece::Rook,
        Piece::Queen,
    ] {
        for square in board.colored_pieces(color, piece) {
            attackers += i32::from(
                attacks_from(piece, square, color, occupied)
                    .into_iter()
                    .any(|attacked| attacked == target),
            );
        }
    }
    attackers
}

#[cfg(test)]
fn zone_defenders(
    board: &Board,
    color: Color,
    king_zone: cozy_chess::BitBoard,
    occupied: cozy_chess::BitBoard,
) -> i32 {
    let mut defenders = 0;
    for piece in [
        Piece::Pawn,
        Piece::Knight,
        Piece::Bishop,
        Piece::Rook,
        Piece::Queen,
    ] {
        for square in board.colored_pieces(color, piece) {
            defenders +=
                i32::from(!(attacks_from(piece, square, color, occupied) & king_zone).is_empty());
        }
    }
    defenders
}

fn attacks_from(
    piece: Piece,
    square: Square,
    color: Color,
    occupied: cozy_chess::BitBoard,
) -> cozy_chess::BitBoard {
    match piece {
        Piece::Pawn => get_pawn_attacks(square, color),
        Piece::Knight => get_knight_moves(square),
        Piece::Bishop => get_bishop_moves(square, occupied),
        Piece::Rook => get_rook_moves(square, occupied),
        Piece::Queen => get_bishop_moves(square, occupied) | get_rook_moves(square, occupied),
        Piece::King => get_king_moves(square),
    }
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

#[cfg(test)]
mod tests {
    use cozy_chess::{Board, Color, Move};

    use super::{attacking_features, reference_attacking_features};

    fn assert_matches_reference(board: &Board) {
        let cached = attacking_features(board);
        for color in [Color::White, Color::Black] {
            assert_eq!(
                cached[color as usize],
                reference_attacking_features(board, color),
                "attack features differ for {color:?} in {board}"
            );
        }
    }

    #[test]
    fn cached_attack_maps_match_reference_positions() {
        for fen in [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r1bq1rk1/ppp2ppp/2n2n2/2b1p1NQ/2B1P3/2NP4/PPP2PPP/R4RK1 w - - 0 10",
            "2kr3r/pppq1ppp/2n1bn2/3p4/3P4/2P1PN2/PP1N1PPP/R2Q1RK1 w - - 0 10",
            "6k1/5ppp/8/8/6P1/8/5P1P/6K1 w - - 0 1",
        ] {
            assert_matches_reference(&fen.parse().unwrap());
        }
    }

    #[test]
    fn cached_attack_maps_match_reference_playout() {
        let mut board = Board::default();
        for turn in 0..128_usize {
            assert_matches_reference(&board);
            let mut moves = Vec::<Move>::new();
            board.generate_moves(|piece_moves| {
                moves.extend(piece_moves);
                false
            });
            if moves.is_empty() {
                board = Board::default();
                continue;
            }
            let chess_move = moves[(turn * 37 + 11) % moves.len()];
            board.play_unchecked(chess_move);
        }
    }
}
