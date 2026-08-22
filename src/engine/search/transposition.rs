use std::fmt::{self, Display, Formatter};
use std::mem::size_of;

use cozy_chess::{Board, Move};

use super::{MATE_THRESHOLD, Score};
use crate::engine::position::repetition_key;

pub(in crate::engine) const DEFAULT_HASH_MIB: usize = 16;
pub(in crate::engine) const MIN_HASH_MIB: usize = 1;
pub(in crate::engine) const MAX_HASH_MIB: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Bound {
    Exact,
    Lower,
    Upper,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Entry {
    key: u64,
    halfmove_clock: u8,
    depth: u32,
    score: Score,
    bound: Bound,
    best_move: Option<Move>,
    generation: u16,
}

impl Entry {
    pub(super) fn depth(self) -> u32 {
        self.depth
    }

    pub(super) fn score_at_ply(self, ply: u32) -> Score {
        score_from_table(self.score, ply)
    }

    pub(super) fn bound(self) -> Bound {
        self.bound
    }

    pub(super) fn best_move(self) -> Option<Move> {
        self.best_move
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine) struct AllocationError;

impl Display for AllocationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("unable to allocate the requested transposition table")
    }
}

#[derive(Debug)]
pub(in crate::engine) struct TranspositionTable {
    entries: Box<[Option<Entry>]>,
    size_mib: usize,
    generation: u16,
    evaluation_profile: Option<u8>,
}

impl TranspositionTable {
    pub(in crate::engine) fn new(size_mib: usize) -> Result<Self, AllocationError> {
        let bytes = size_mib.checked_mul(1024 * 1024).ok_or(AllocationError)?;
        let maximum_entries = bytes / size_of::<Option<Entry>>();
        let entry_count = floor_power_of_two(maximum_entries).ok_or(AllocationError)?;
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(entry_count)
            .map_err(|_| AllocationError)?;
        entries.resize(entry_count, None);

        Ok(Self {
            entries: entries.into_boxed_slice(),
            size_mib,
            generation: 0,
            evaluation_profile: None,
        })
    }

    pub(in crate::engine) fn size_mib(&self) -> usize {
        self.size_mib
    }

    pub(super) fn start_search(&mut self, evaluation_profile: u8) {
        if self.evaluation_profile != Some(evaluation_profile) {
            self.entries.fill(None);
            self.evaluation_profile = Some(evaluation_profile);
        }
        self.generation = self.generation.wrapping_add(1);
    }

    pub(in crate::engine) fn clear(&mut self) {
        self.entries.fill(None);
        self.generation = self.generation.wrapping_add(1);
    }

    pub(super) fn probe(&self, board: &Board) -> Option<Entry> {
        let key = repetition_key(board);
        self.entries[self.index(key)]
            .filter(|entry| entry.key == key && entry.halfmove_clock == board.halfmove_clock())
    }

    pub(super) fn store(
        &mut self,
        board: &Board,
        depth: u32,
        ply: u32,
        score: Score,
        bound: Bound,
        best_move: Option<Move>,
    ) {
        let key = repetition_key(board);
        let index = self.index(key);
        let candidate = Entry {
            key,
            halfmove_clock: board.halfmove_clock(),
            depth,
            score: score_to_table(score, ply),
            bound,
            best_move,
            generation: self.generation,
        };

        if let Some(existing) = &mut self.entries[index] {
            let same_position =
                existing.key == key && existing.halfmove_clock == candidate.halfmove_clock;
            if same_position && existing.depth > depth && existing.bound == Bound::Exact {
                existing.generation = self.generation;
                if existing.best_move.is_none() {
                    existing.best_move = best_move;
                }
                return;
            }
            if !same_position && existing.generation == self.generation && existing.depth > depth {
                return;
            }
        }

        self.entries[index] = Some(candidate);
    }

    pub(super) fn write_principal_variation(
        &self,
        board: &Board,
        depth: u32,
        output: &mut Vec<Move>,
    ) {
        output.clear();
        let mut board = board.clone();

        for _ in 0..depth {
            let Some(entry) = self.probe(&board) else {
                break;
            };
            if entry.bound != Bound::Exact {
                break;
            }
            let Some(best_move) = entry.best_move else {
                break;
            };
            if !board.is_legal(best_move) {
                break;
            }

            output.push(best_move);
            board.play_unchecked(best_move);
        }
    }

    fn index(&self, key: u64) -> usize {
        key as usize & (self.entries.len() - 1)
    }
}

fn floor_power_of_two(value: usize) -> Option<usize> {
    value
        .checked_next_power_of_two()
        .map(|power| if power == value { power } else { power / 2 })
}

fn score_to_table(score: Score, ply: u32) -> Score {
    if score >= MATE_THRESHOLD {
        score + ply as Score
    } else if score <= -MATE_THRESHOLD {
        score - ply as Score
    } else {
        score
    }
}

fn score_from_table(score: Score, ply: u32) -> Score {
    if score >= MATE_THRESHOLD {
        score - ply as Score
    } else if score <= -MATE_THRESHOLD {
        score + ply as Score
    } else {
        score
    }
}

#[cfg(test)]
mod tests {
    use super::{Bound, MATE_THRESHOLD, TranspositionTable};
    use crate::engine::Position;

    #[test]
    fn stores_and_recovers_entries() {
        let position = Position::default();
        let best_move = position.search_moves()[0];
        let mut table = TranspositionTable::new(1).unwrap();

        table.store(position.board(), 6, 0, 42, Bound::Exact, Some(best_move));
        let entry = table.probe(position.board()).unwrap();

        assert_eq!(entry.depth(), 6);
        assert_eq!(entry.score_at_ply(0), 42);
        assert_eq!(entry.bound(), Bound::Exact);
        assert_eq!(entry.best_move(), Some(best_move));
    }

    #[test]
    fn isolates_positions_with_different_halfmove_clocks() {
        let first = Position::from_fen("7k/8/8/8/8/8/R7/K7 w - - 0 1").unwrap();
        let second = Position::from_fen("7k/8/8/8/8/8/R7/K7 w - - 99 50").unwrap();
        let mut table = TranspositionTable::new(1).unwrap();

        table.store(first.board(), 4, 0, 75, Bound::Exact, None);

        assert!(table.probe(second.board()).is_none());
    }

    #[test]
    fn normalizes_mate_scores_across_root_plies() {
        let position = Position::default();
        let mut table = TranspositionTable::new(1).unwrap();

        table.store(
            position.board(),
            8,
            7,
            MATE_THRESHOLD + 20,
            Bound::Exact,
            None,
        );
        let entry = table.probe(position.board()).unwrap();

        assert_eq!(entry.score_at_ply(3), MATE_THRESHOLD + 24);
    }

    #[test]
    fn clear_discards_entries() {
        let position = Position::default();
        let mut table = TranspositionTable::new(1).unwrap();
        table.store(position.board(), 1, 0, 0, Bound::Upper, None);

        table.clear();

        assert!(table.probe(position.board()).is_none());
    }
    #[test]
    fn evaluation_profile_changes_discard_entries() {
        let position = Position::default();
        let mut table = TranspositionTable::new(1).unwrap();
        table.start_search(0);
        table.store(position.board(), 1, 0, 0, Bound::Upper, None);

        table.start_search(0);
        assert!(table.probe(position.board()).is_some());

        table.start_search(100);
        assert!(table.probe(position.board()).is_none());
    }
}
