use cozy_chess::{Board, Color, Move, Piece, Square};

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

#[derive(Clone, Debug)]
pub(in crate::engine) struct ExchangeOutcome {
    pub(in crate::engine) target: Square,
    pub(in crate::engine) line: Vec<Move>,
    pub(in crate::engine) final_board: Board,
    pub(in crate::engine) material_balance: Score,
    pub(in crate::engine) truncated: bool,
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

pub(in crate::engine) fn exchange_risk_on(
    board: &Board,
    mover: Color,
    target: cozy_chess::Square,
) -> Score {
    let Some(oriented) = orient_to(board, !mover) else {
        return 0;
    };
    let before = material_balance(&oriented, mover);
    let mut worst = before;
    for chess_move in generate_moves(&oriented).into_iter().filter(|&chess_move| {
        chess_move.to == target && captured_piece(&oriented, chess_move).is_some()
    }) {
        let mut child = oriented.clone();
        child.play_unchecked(chess_move);
        worst = worst.min(exchange_value(
            &child,
            mover,
            target,
            MAX_EXCHANGE_PLIES - 1,
        ));
    }
    (before - worst).max(0)
}

pub(in crate::engine) fn exchange_outcome(
    board: &Board,
    mover: Color,
    target: Square,
) -> ExchangeOutcome {
    exchange_outcome_with_limit(board, mover, target, MAX_EXCHANGE_PLIES)
}

pub(in crate::engine) fn material_balance_after_exchange(
    board: &Board,
    mover: Color,
    target: Square,
) -> Score {
    let outcome = exchange_outcome(board, mover, target);
    debug_assert_eq!(outcome.target, target);
    debug_assert_eq!(
        outcome.material_balance,
        material_balance(&outcome.final_board, mover)
    );
    debug_assert!(outcome.line.len() <= usize::from(MAX_EXCHANGE_PLIES));
    debug_assert!(!outcome.truncated || outcome.line.len() == usize::from(MAX_EXCHANGE_PLIES));
    outcome.material_balance
}

fn exchange_value(board: &Board, perspective: Color, target: Square, remaining: u8) -> Score {
    exchange_outcome_with_limit(board, perspective, target, remaining).material_balance
}

fn exchange_outcome_with_limit(
    board: &Board,
    perspective: Color,
    target: Square,
    remaining: u8,
) -> ExchangeOutcome {
    let current = material_balance(board, perspective);
    let captures = generate_moves(board)
        .into_iter()
        .filter(|chess_move| {
            chess_move.to == target && captured_piece(board, *chess_move).is_some()
        })
        .collect::<Vec<_>>();
    if remaining == 0 || captures.is_empty() {
        return ExchangeOutcome {
            target,
            line: Vec::new(),
            final_board: board.clone(),
            material_balance: current,
            truncated: remaining == 0 && !captures.is_empty(),
        };
    }

    let maximizing = board.side_to_move() == perspective;
    let mut best = ExchangeOutcome {
        target,
        line: Vec::new(),
        final_board: board.clone(),
        material_balance: current,
        truncated: false,
    };
    for chess_move in captures {
        let mut child = board.clone();
        child.play_unchecked(chess_move);
        let mut candidate = exchange_outcome_with_limit(&child, perspective, target, remaining - 1);
        candidate.line.insert(0, chess_move);
        let improves = if maximizing {
            candidate.material_balance > best.material_balance
        } else {
            candidate.material_balance < best.material_balance
        };
        if improves {
            best = candidate;
        }
    }
    best
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
    use cozy_chess::{Board, Color, Piece, Square};

    use super::{
        exchange_outcome, exchange_outcome_with_limit, material_balance, tactical_snapshot,
    };

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

    #[test]
    fn exchange_risk_values_promotion_captures() {
        let board: Board = "4k3/8/8/8/8/8/6p1/K6R b - - 0 1".parse().unwrap();
        let snapshot = tactical_snapshot(&board, Color::White);

        assert_eq!(snapshot.exchange_risk, 1_300);
    }

    #[test]
    fn exchange_risk_ignores_pinned_attackers() {
        let board: Board = "4k3/4p3/3Q4/8/8/8/8/K3R3 b - - 0 1".parse().unwrap();
        let snapshot = tactical_snapshot(&board, Color::White);

        assert_eq!(snapshot.exchange_risk, 0);
    }

    #[test]
    fn exchange_outcome_keeps_the_selected_line_and_board_together() {
        let board: Board = "4k3/8/8/3r4/8/8/3Q4/4K3 b - - 0 1".parse().unwrap();

        let outcome = exchange_outcome(&board, Color::White, Square::D2);

        assert_eq!(outcome.target, Square::D2);
        assert_eq!(outcome.material_balance, 0);
        assert_eq!(outcome.line.len(), 2);
        assert_eq!(outcome.line[0].from, Square::D5);
        assert_eq!(outcome.line[0].to, Square::D2);
        assert_eq!(outcome.line[1].from, Square::E1);
        assert_eq!(outcome.line[1].to, Square::D2);
        assert_eq!(outcome.final_board.piece_on(Square::D2), Some(Piece::King));
        assert_eq!(outcome.final_board.color_on(Square::D2), Some(Color::White));
        assert!(!outcome.truncated);
    }

    #[test]
    fn exchange_outcome_records_en_passant_on_the_destination_square() {
        let board: Board = "4k3/8/8/8/3pP3/8/8/4K3 b - e3 0 1".parse().unwrap();

        let outcome = exchange_outcome(&board, Color::White, Square::E3);

        assert_eq!(outcome.material_balance, -100);
        assert_eq!(outcome.line.len(), 1);
        assert_eq!(outcome.final_board.piece_on(Square::E3), Some(Piece::Pawn));
        assert_eq!(outcome.final_board.color_on(Square::E3), Some(Color::Black));
        assert_eq!(outcome.final_board.piece_on(Square::E4), None);
        assert!(!outcome.truncated);
    }

    #[test]
    fn exchange_outcome_keeps_the_best_promotion() {
        let board: Board = "4k3/8/8/8/8/8/6p1/4K2R b - - 0 1".parse().unwrap();

        let outcome = exchange_outcome(&board, Color::White, Square::H1);

        assert_eq!(outcome.material_balance, -900);
        assert_eq!(outcome.line.len(), 1);
        assert_eq!(outcome.line[0].promotion, Some(Piece::Queen));
        assert_eq!(outcome.final_board.piece_on(Square::H1), Some(Piece::Queen));
        assert_eq!(outcome.final_board.color_on(Square::H1), Some(Color::Black));
        assert!(!outcome.truncated);
    }

    #[test]
    fn exchange_outcome_marks_an_unresolved_depth_boundary() {
        let board: Board = "3r3k/8/8/8/3Q4/8/8/3R3K b - - 0 1".parse().unwrap();

        let truncated = exchange_outcome_with_limit(&board, Color::White, Square::D4, 1);
        assert!(truncated.truncated);
        assert_eq!(truncated.line.len(), 1);
        assert_eq!(truncated.line[0].from, Square::D8);
        assert_eq!(truncated.line[0].to, Square::D4);
        assert_eq!(
            truncated.final_board.piece_on(Square::D4),
            Some(Piece::Rook)
        );
        assert_eq!(truncated.material_balance, 0);

        let settled = exchange_outcome(&board, Color::White, Square::D4);
        assert!(!settled.truncated);
        assert_eq!(settled.line.len(), 2);
        assert_eq!(settled.line[1].from, Square::D1);
        assert_eq!(settled.line[1].to, Square::D4);
        assert_eq!(settled.final_board.piece_on(Square::D4), Some(Piece::Rook));
        assert_eq!(settled.final_board.color_on(Square::D4), Some(Color::White));
        assert_eq!(settled.material_balance, 500);
    }
}
