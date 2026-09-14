use std::cell::Cell;

use cozy_chess::{Board, Color, Piece};

use super::Score;

/// Each entry is 64 bytes; the per-thread table occupies 512 KiB.
const CACHE_SLOTS: usize = 8192;

#[derive(Clone, Copy, Default)]
struct Entry {
    pieces: [u64; 6],
    white: u64,
    state: u32,
    value: Score,
}

impl Entry {
    fn position(board: &Board) -> Self {
        let mut state = board.side_to_move() as u32;
        for color in Color::ALL {
            let rights = board.castle_rights(color);
            let short = rights.short.map_or(0, |file| file as u32 + 1);
            let long = rights.long.map_or(0, |file| file as u32 + 1);
            state |= (short | long << 4) << (1 + 8 * color as u32);
        }
        Self {
            pieces: std::array::from_fn(|index| board.pieces(Piece::ALL[index]).0),
            white: board.colors(Color::White).0,
            state,
            value: 0,
        }
    }

    fn matches(self, other: Self) -> bool {
        self.state == other.state && self.white == other.white && self.pieces == other.pieces
    }
}

struct Memo {
    entries: Box<[Cell<Entry>]>,
}

impl Memo {
    fn new(slots: usize) -> Self {
        assert!(slots.is_power_of_two());
        Self {
            entries: (0..slots).map(|_| Cell::new(Entry::default())).collect(),
        }
    }

    fn evaluate(&self, board: &Board, compute: impl FnOnce() -> Score) -> Score {
        let slot = board.hash() as usize & (self.entries.len() - 1);
        let mut position = Entry::position(board);
        let previous = self.entries[slot].get();
        if previous.matches(position) {
            return previous.value;
        }
        position.value = compute();
        self.entries[slot].set(position);
        position.value
    }
}

thread_local! {
    static MEMO: Memo = Memo::new(CACHE_SLOTS);
}

/// Memoizes only the objective static score, never a search result or a draw.
///
/// The hash selects a slot; every hit verifies the complete score inputs.
/// The evaluator uses piece placement, colors, tempo and castling rights, but
/// not en passant, clocks, repetition history or aggression. The all-empty
/// initial entry cannot match a board containing the two kings.
#[inline]
pub(super) fn evaluate(board: &Board, compute: impl FnOnce() -> Score) -> Score {
    MEMO.with(|memo| memo.evaluate(board, compute))
}

#[cfg(test)]
mod tests {
    use super::{Entry, Memo};
    use crate::engine::evaluation::{
        EvaluationConfig, evaluate_with_config, evaluate_with_trace_and_config,
    };
    use cozy_chess::{Board, Color};
    use std::cell::Cell;

    #[test]
    fn entries_fit_one_cache_line() {
        assert_eq!(std::mem::size_of::<Entry>(), 64);
    }

    #[test]
    fn an_exact_repeat_reuses_the_value() {
        let board = Board::default();
        let memo = Memo::new(1);
        assert_eq!(memo.evaluate(&board, || 47), 47);
        assert_eq!(memo.evaluate(&board, || panic!("unexpected miss")), 47);
    }

    #[test]
    fn collisions_verify_pieces_colors_tempo_and_castling() {
        let memo = Memo::new(1);
        let calls = Cell::new(0);
        let positions = [
            "r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1",
            "r3k2r/8/8/8/8/8/8/R3K2R b KQkq - 0 1",
            "r3k2r/8/8/8/8/8/8/R3K2R b Qkq - 0 1",
            "r3k2r/8/8/8/8/8/8/R3K2R b Qq - 0 1",
            "r3k2r/8/8/8/8/8/N7/R3K2R b Qq - 0 1",
            "r3k2r/8/8/8/8/8/B7/R3K2R b Qq - 0 1",
            "r3k2r/8/8/8/8/8/b7/R3K2R b Qq - 0 1",
            "r3k2r/8/8/8/8/8/1b6/R3K2R b Qq - 0 1",
        ];
        for (index, fen) in positions.into_iter().enumerate() {
            let board: Board = fen.parse().unwrap();
            let expected = index as i32 + 100;
            assert_eq!(
                memo.evaluate(&board, || {
                    calls.set(calls.get() + 1);
                    expected
                }),
                expected,
                "{fen}"
            );
            assert_eq!(
                memo.evaluate(&board, || panic!("unexpected miss")),
                expected
            );
        }
        assert_eq!(calls.get(), positions.len());
        let first: Board = positions[0].parse().unwrap();
        assert_eq!(memo.evaluate(&first, || -31), -31);
    }

    #[test]
    fn clock_and_en_passant_metadata_do_not_change_evaluation_inputs() {
        let original: Board = "4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1".parse().unwrap();
        let changed: Board = "4k3/8/8/3pP3/8/8/8/4K3 w - - 87 90".parse().unwrap();
        assert!(Entry::position(&original).matches(Entry::position(&changed)));
        let memo = Memo::new(1);
        let config = EvaluationConfig::new(0);
        let expected = evaluate_with_trace_and_config(&original, config).blended;
        assert_eq!(
            expected,
            evaluate_with_trace_and_config(&changed, config).blended
        );
        assert_eq!(memo.evaluate(&original, || expected), expected);
        assert_eq!(
            memo.evaluate(&changed, || panic!("unexpected miss")),
            expected
        );
    }

    #[test]
    fn cached_scores_match_uncached_evaluation_over_legal_playouts() {
        for seed in [7_usize, 19, 37, 53, 101, 131, 173, 229] {
            let mut board = Board::default();
            for turn in 0..512 {
                let config = EvaluationConfig::new(0);
                let expected = uncached_score(&board);
                for _ in 0..2 {
                    assert_eq!(evaluate_with_config(&board, config), expected, "{board}");
                }
                if let Some(opposite) = board.null_move() {
                    assert_eq!(
                        evaluate_with_config(&opposite, config),
                        uncached_score(&opposite),
                        "{opposite}"
                    );
                }
                let mut moves = Vec::new();
                board.generate_moves(|batch| {
                    moves.extend(batch);
                    false
                });
                if moves.is_empty() {
                    board = Board::default();
                } else {
                    board.play_unchecked(moves[(turn * seed + 11) % moves.len()]);
                }
            }
        }
    }

    fn uncached_score(board: &Board) -> i32 {
        use crate::engine::evaluation::{features, weights};
        let features = features::extract_with_style(board, false);
        let score = weights::score(&features);
        let phase = features::phase(board);
        let blended = (score.middle_game() * phase + score.end_game() * (24 - phase)) / 24;
        blended
            * if board.side_to_move() == Color::White {
                1
            } else {
                -1
            }
    }
}
