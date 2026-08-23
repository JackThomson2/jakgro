use std::fmt::{self, Display, Formatter};
use std::mem::size_of;

use cozy_chess::{Board, Move};

use super::{MATE_THRESHOLD, Score};
use crate::engine::position::repetition_key;

pub(in crate::engine) const DEFAULT_HASH_MIB: usize = 16;
pub(in crate::engine) const MIN_HASH_MIB: usize = 1;
pub(in crate::engine) const MAX_HASH_MIB: usize = 1024;
const BUCKET_SIZE: usize = 4;
type Bucket = [Option<Entry>; BUCKET_SIZE];

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
    buckets: Box<[Bucket]>,
    size_mib: usize,
    generation: u16,
    evaluation_profile: Option<u8>,
}

impl TranspositionTable {
    pub(in crate::engine) fn new(size_mib: usize) -> Result<Self, AllocationError> {
        let bytes = size_mib.checked_mul(1024 * 1024).ok_or(AllocationError)?;
        let maximum_buckets = bytes / size_of::<Bucket>();
        let bucket_count = floor_power_of_two(maximum_buckets).ok_or(AllocationError)?;
        let mut buckets = Vec::new();
        buckets
            .try_reserve_exact(bucket_count)
            .map_err(|_| AllocationError)?;
        buckets.resize(bucket_count, [None; BUCKET_SIZE]);

        Ok(Self {
            buckets: buckets.into_boxed_slice(),
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
            self.buckets.fill([None; BUCKET_SIZE]);
            self.evaluation_profile = Some(evaluation_profile);
        }
        self.generation = self.generation.wrapping_add(1);
    }

    pub(in crate::engine) fn clear(&mut self) {
        self.buckets.fill([None; BUCKET_SIZE]);
        self.generation = self.generation.wrapping_add(1);
    }

    pub(super) fn probe(&self, board: &Board) -> Option<Entry> {
        self.probe_key(repetition_key(board), board.halfmove_clock())
    }

    #[cfg(test)]
    pub(super) fn store(
        &mut self,
        board: &Board,
        depth: u32,
        ply: u32,
        score: Score,
        bound: Bound,
        best_move: Option<Move>,
    ) {
        self.store_key(
            repetition_key(board),
            board.halfmove_clock(),
            depth,
            ply,
            score,
            bound,
            best_move,
        );
    }
    pub(super) fn probe_key(&self, key: u64, halfmove_clock: u8) -> Option<Entry> {
        self.buckets[self.index(key)]
            .iter()
            .flatten()
            .find(|entry| entry.key == key && entry.halfmove_clock == halfmove_clock)
            .copied()
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn store_key(
        &mut self,
        key: u64,
        halfmove_clock: u8,
        depth: u32,
        ply: u32,
        score: Score,
        bound: Bound,
        best_move: Option<Move>,
    ) {
        self.store_entry(Entry {
            key,
            halfmove_clock,
            depth,
            score: score_to_table(score, ply),
            bound,
            best_move,
            generation: self.generation,
        });
    }

    fn store_entry(&mut self, candidate: Entry) {
        let index = self.index(candidate.key);
        let generation = self.generation;
        let bucket = &mut self.buckets[index];
        if let Some(existing) = bucket.iter_mut().flatten().find(|entry| {
            entry.key == candidate.key && entry.halfmove_clock == candidate.halfmove_clock
        }) {
            if existing.depth > candidate.depth && existing.bound == Bound::Exact {
                existing.generation = self.generation;
                if existing.best_move.is_none() {
                    existing.best_move = candidate.best_move;
                }
                return;
            }
            *existing = candidate;
            return;
        }

        if let Some(empty) = bucket.iter_mut().find(|entry| entry.is_none()) {
            *empty = Some(candidate);
            return;
        }

        let replacement = replacement_index(bucket, generation);
        let existing = bucket[replacement].expect("a full bucket has a replacement entry");
        if existing.generation == generation
            && candidate.bound != Bound::Exact
            && (existing.bound == Bound::Exact || existing.depth > candidate.depth)
        {
            return;
        }
        bucket[replacement] = Some(candidate);
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
        key as usize & (self.buckets.len() - 1)
    }
}

fn replacement_index(bucket: &Bucket, generation: u16) -> usize {
    if let Some((index, _)) = bucket
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| entry.map(|entry| (index, entry)))
        .filter(|(_, entry)| entry.generation != generation)
        .max_by_key(|(_, entry)| {
            (
                generation.wrapping_sub(entry.generation),
                entry.bound != Bound::Exact,
                u32::MAX - entry.depth,
            )
        })
    {
        return index;
    }

    bucket
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| entry.map(|entry| (index, entry)))
        .filter(|(_, entry)| entry.bound != Bound::Exact)
        .min_by_key(|(_, entry)| entry.depth)
        .or_else(|| {
            bucket
                .iter()
                .enumerate()
                .filter_map(|(index, entry)| entry.map(|entry| (index, entry)))
                .min_by_key(|(_, entry)| entry.depth)
        })
        .map(|(index, _)| index)
        .expect("a full bucket has an occupied entry")
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
    fn synthetic_entry(key: u64, depth: u32, bound: Bound, generation: u16) -> super::Entry {
        super::Entry {
            key,
            halfmove_clock: 0,
            depth,
            score: 0,
            bound,
            best_move: None,
            generation,
        }
    }

    #[test]
    fn colliding_entries_share_a_bucket() {
        let mut table = TranspositionTable::new(1).unwrap();
        table.start_search(0);
        let stride = table.buckets.len() as u64;

        for slot in 0..super::BUCKET_SIZE {
            table.store_entry(synthetic_entry(
                slot as u64 * stride,
                slot as u32 + 1,
                Bound::Lower,
                table.generation,
            ));
        }

        for slot in 0..super::BUCKET_SIZE {
            assert!(table.probe_key(slot as u64 * stride, 0).is_some());
        }
    }
    #[test]
    fn bucket_allocation_stays_within_the_requested_size() {
        let table = TranspositionTable::new(1).unwrap();
        let bytes = table.buckets.len() * std::mem::size_of::<super::Bucket>();

        assert!(bytes <= 1024 * 1024);
        assert!(bytes * 2 > 1024 * 1024);
    }

    #[test]
    fn replacement_prefers_the_shallowest_non_exact_entry() {
        let mut table = TranspositionTable::new(1).unwrap();
        table.start_search(0);
        let stride = table.buckets.len() as u64;
        let entries = [
            (0, 8, Bound::Exact),
            (1, 6, Bound::Exact),
            (2, 4, Bound::Lower),
            (3, 2, Bound::Upper),
        ];
        for (slot, depth, bound) in entries {
            table.store_entry(synthetic_entry(
                slot * stride,
                depth,
                bound,
                table.generation,
            ));
        }

        table.store_entry(synthetic_entry(
            4 * stride,
            3,
            Bound::Lower,
            table.generation,
        ));

        assert!(table.probe_key(3 * stride, 0).is_none());
        assert!(table.probe_key(4 * stride, 0).is_some());
        assert_eq!(table.probe_key(0, 0).unwrap().depth(), 8);
    }

    #[test]
    fn stale_entries_yield_to_the_current_generation() {
        let mut table = TranspositionTable::new(1).unwrap();
        table.start_search(0);
        let stride = table.buckets.len() as u64;
        for slot in 0..super::BUCKET_SIZE {
            table.store_entry(synthetic_entry(
                slot as u64 * stride,
                8,
                Bound::Exact,
                table.generation,
            ));
        }

        table.start_search(0);
        table.store_entry(synthetic_entry(
            4 * stride,
            1,
            Bound::Upper,
            table.generation,
        ));

        assert!(table.probe_key(4 * stride, 0).is_some());
    }

    #[test]
    fn a_deeper_matching_exact_entry_is_preserved() {
        let mut table = TranspositionTable::new(1).unwrap();
        table.start_search(0);
        let key = 7;
        table.store_entry(synthetic_entry(key, 8, Bound::Exact, table.generation));
        table.store_entry(synthetic_entry(key, 2, Bound::Lower, table.generation));

        let entry = table.probe_key(key, 0).unwrap();
        assert_eq!(entry.depth(), 8);
        assert_eq!(entry.bound(), Bound::Exact);
        assert_eq!(entry.generation, table.generation);
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
