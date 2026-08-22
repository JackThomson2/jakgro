use cozy_chess::{Board, Color, Move, Piece};

use super::{Score, features, piece_value};

const MAX_EXCHANGE_PLIES: u8 = 8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::engine) struct StyleSnapshot {
    pub(in crate::engine) material_balance: Score,
    pub(in crate::engine) attack_momentum: Score,
    pub(in crate::engine) own_king_danger: Score,
    pub(in crate::engine) attackers: Score,
    pub(in crate::engine) attacker_variety: Score,
    pub(in crate::engine) coordination: Score,
    pub(in crate::engine) supported_threats: Score,
    pub(in crate::engine) open_lines: Score,
    pub(in crate::engine) defender_shortage: Score,
    pub(in crate::engine) pawn_breaks: Score,
    pub(in crate::engine) mover_queens: Score,
    pub(in crate::engine) total_queens: Score,
    pub(in crate::engine) king_pressure_advantage: Score,
    pub(in crate::engine) pawn_storm_advantage: Score,
    pub(in crate::engine) threat_advantage: Score,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::engine) struct TacticalSnapshot {
    pub(in crate::engine) style: StyleSnapshot,
    pub(in crate::engine) legal_checks: Score,
    pub(in crate::engine) exchange_risk: Score,
}

pub(in crate::engine) fn style_snapshot(board: &Board, mover: Color) -> StyleSnapshot {
    let features = features::extract(board);
    let (attack, enemy_attack, sign) = if mover == Color::White {
        (features.white_attack, features.black_attack, 1)
    } else {
        (features.black_attack, features.white_attack, -1)
    };

    StyleSnapshot {
        material_balance: material_balance(board, mover),
        attack_momentum: attack.compensation_pressure(),
        own_king_danger: enemy_attack.compensation_pressure(),
        attackers: attack.attackers,
        attacker_variety: attack.attacker_variety,
        coordination: attack.coordination(),
        supported_threats: attack.supported_threats,
        open_lines: attack.open_lines,
        defender_shortage: attack.defender_shortage,
        pawn_breaks: attack.pawn_breaks,
        mover_queens: board.colored_pieces(mover, Piece::Queen).len() as Score,
        total_queens: board.pieces(Piece::Queen).len() as Score,
        king_pressure_advantage: sign * features.king_pressure,
        pawn_storm_advantage: sign * features.pawn_storm,
        threat_advantage: sign * features.threats,
    }
}

pub(in crate::engine) fn tactical_snapshot(board: &Board, mover: Color) -> TacticalSnapshot {
    TacticalSnapshot {
        style: style_snapshot(board, mover),
        legal_checks: legal_check_count(board, mover),
        exchange_risk: exchange_risk(board, mover),
    }
}

pub(in crate::engine) fn material_balance(board: &Board, perspective: Color) -> Score {
    [
        Piece::Pawn,
        Piece::Knight,
        Piece::Bishop,
        Piece::Rook,
        Piece::Queen,
    ]
    .into_iter()
    .map(|piece| {
        piece_value(piece)
            * (board.colored_pieces(perspective, piece).len() as Score
                - board.colored_pieces(!perspective, piece).len() as Score)
    })
    .sum()
}

fn legal_check_count(board: &Board, color: Color) -> Score {
    let Some(oriented) = orient_to(board, color) else {
        return 0;
    };
    generate_moves(&oriented)
        .into_iter()
        .filter(|&chess_move| {
            let mut child = oriented.clone();
            child.play_unchecked(chess_move);
            !child.checkers().is_empty()
        })
        .count() as Score
}

fn exchange_risk(board: &Board, mover: Color) -> Score {
    let Some(oriented) = orient_to(board, !mover) else {
        return 0;
    };
    let before = material_balance(&oriented, mover);
    let mut worst = before;
    for chess_move in generate_moves(&oriented)
        .into_iter()
        .filter(|&chess_move| captured_piece(&oriented, chess_move).is_some())
    {
        let mut child = oriented.clone();
        child.play_unchecked(chess_move);
        worst = worst.min(exchange_value(
            &child,
            mover,
            chess_move.to,
            MAX_EXCHANGE_PLIES - 1,
        ));
    }
    (before - worst).max(0)
}

fn exchange_value(
    board: &Board,
    perspective: Color,
    target: cozy_chess::Square,
    remaining: u8,
) -> Score {
    let current = material_balance(board, perspective);
    if remaining == 0 {
        return current;
    }
    let captures = generate_moves(board)
        .into_iter()
        .filter(|chess_move| {
            chess_move.to == target && captured_piece(board, *chess_move).is_some()
        })
        .collect::<Vec<_>>();
    if captures.is_empty() {
        return current;
    }

    if board.side_to_move() == perspective {
        captures.into_iter().fold(current, |best, chess_move| {
            let mut child = board.clone();
            child.play_unchecked(chess_move);
            best.max(exchange_value(&child, perspective, target, remaining - 1))
        })
    } else {
        captures.into_iter().fold(current, |best, chess_move| {
            let mut child = board.clone();
            child.play_unchecked(chess_move);
            best.min(exchange_value(&child, perspective, target, remaining - 1))
        })
    }
}

fn orient_to(board: &Board, color: Color) -> Option<Board> {
    if board.side_to_move() == color {
        Some(board.clone())
    } else {
        board.null_move()
    }
}

fn generate_moves(board: &Board) -> Vec<Move> {
    let mut moves = Vec::new();
    board.generate_moves(|piece_moves| {
        moves.extend(piece_moves);
        false
    });
    moves
}

fn captured_piece(board: &Board, chess_move: Move) -> Option<Piece> {
    board.piece_on(chess_move.to).or_else(|| {
        (board.piece_on(chess_move.from) == Some(Piece::Pawn)
            && chess_move.from.file() != chess_move.to.file())
        .then_some(Piece::Pawn)
    })
}

#[cfg(test)]
mod tests {
    use cozy_chess::{Board, Color};

    use super::{material_balance, tactical_snapshot};

    #[test]
    fn material_balance_is_mover_relative() {
        let board: Board = "4k3/8/8/8/8/8/3Q4/4K3 w - - 0 1".parse().unwrap();

        assert_eq!(material_balance(&board, Color::White), 900);
        assert_eq!(material_balance(&board, Color::Black), -900);
    }

    #[test]
    fn tactical_snapshot_counts_legal_checks() {
        let board: Board = "7k/8/8/8/8/8/4Q3/4K3 w - - 0 1".parse().unwrap();
        let snapshot = tactical_snapshot(&board, Color::White);

        assert!(snapshot.legal_checks > 0);
        assert_eq!(snapshot.exchange_risk, 0);
    }

    #[test]
    fn exchange_risk_finds_a_hanging_rook() {
        let board: Board = "4k3/8/8/3q4/8/8/3R4/K7 b - - 0 1".parse().unwrap();
        let snapshot = tactical_snapshot(&board, Color::White);

        assert_eq!(snapshot.exchange_risk, 500);
    }

    #[test]
    fn exchange_risk_accounts_for_a_legal_recapture() {
        let board: Board = "4k3/8/8/3q4/8/8/3R4/4K3 b - - 0 1".parse().unwrap();
        let snapshot = tactical_snapshot(&board, Color::White);

        assert_eq!(snapshot.exchange_risk, 0);
    }

    #[test]
    fn exchange_risk_recognizes_en_passant() {
        let board: Board = "4k3/8/8/8/3pP3/8/8/4K3 b - e3 0 1".parse().unwrap();
        let snapshot = tactical_snapshot(&board, Color::White);

        assert_eq!(snapshot.exchange_risk, 100);
    }
}
