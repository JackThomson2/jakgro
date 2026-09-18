use cozy_chess::{Board, Color, Move};

use super::NeuralEvaluator;
use crate::engine::evaluation::{MAX_PLY, nnue::Network};
use crate::engine::nnue_test_support::network_bytes;

#[test]
fn lazy_rows_match_refresh_after_skipped_parents_siblings_and_null_moves() {
    let network = Network::from_bytes(&network_bytes(0, true)).unwrap();
    let mut evaluator = NeuralEvaluator::new(&network);
    let mut board = Board::default();
    for turn in 0..192 {
        let ply = [0, 7, 3, MAX_PLY, 7, 1, 64][turn % 7];
        assert_eq!(
            evaluator.evaluate(&board, ply),
            network.evaluate(&board),
            "{board}"
        );
        let reference = network.accumulator(&board);
        for side in [Color::White, Color::Black] {
            assert_eq!(
                evaluator.stack.values(ply as usize, side),
                reference.values(side)
            );
        }
        if let Some(null) = board.null_move() {
            assert_eq!(evaluator.evaluate(&null, ply), network.evaluate(&null));
            assert_eq!(evaluator.evaluate(&board, ply), network.evaluate(&board));
        }
        let mut moves = Vec::new();
        board.generate_moves(|batch| {
            moves.extend(batch);
            false
        });
        if moves.is_empty() {
            board = Board::default();
        } else {
            let mut sibling = board.clone();
            sibling.play_unchecked(moves[(turn * 37 + 11) % moves.len()]);
            assert_eq!(
                evaluator.evaluate(&sibling, ply),
                network.evaluate(&sibling)
            );
            board.play_unchecked(moves[(turn * 19 + 7) % moves.len()]);
        }
    }
    assert_eq!(
        evaluator.evaluate(&board, u32::MAX),
        network.evaluate(&board)
    );
}

#[test]
fn worker_rows_cover_special_moves_and_king_bucket_refreshes() {
    let network = Network::from_bytes(&network_bytes(0, true)).unwrap();
    let mut evaluator = NeuralEvaluator::new(&network);
    for (fen, text) in [
        ("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1", "e1h1"),
        ("r3k2r/8/8/8/8/8/8/R3K2R b KQkq - 0 1", "e8a8"),
        ("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1", "e5d6"),
        ("4k3/8/8/8/3Pp3/8/8/4K3 b - d3 0 1", "e4d3"),
        ("1r5k/P7/8/8/8/8/8/K7 w - - 0 1", "a7b8n"),
        ("7k/8/8/8/8/8/8/4K3 w - - 0 1", "e1d1"),
    ] {
        let board: Board = fen.parse().unwrap();
        let chess_move: Move = text.parse().unwrap();
        assert!(board.is_legal(chess_move), "{fen}: {text}");
        let mut child = board.clone();
        child.play_unchecked(chess_move);
        for position in [&board, &child, &board, &child] {
            assert_eq!(evaluator.evaluate(position, 5), network.evaluate(position));
        }
    }
}

#[test]
fn independent_workers_share_only_the_immutable_network() {
    let network = Network::from_bytes(&network_bytes(0, true)).unwrap();
    let mut first = NeuralEvaluator::new(&network);
    let mut second = NeuralEvaluator::new(&network);
    let board = Board::default();
    let other: Board = "4k3/8/8/8/8/8/4P3/4K3 b - - 0 1".parse().unwrap();
    first.evaluate(&board, 3);
    let before = first.stack.values(3, Color::White);
    second.evaluate(&other, 3);
    assert_eq!(first.stack.values(3, Color::White), before);
    assert_eq!(first.evaluate(&board, 3), network.evaluate(&board));
    assert_eq!(second.evaluate(&other, 3), network.evaluate(&other));
}
