use std::fmt::{self, Display, Formatter};
use std::mem::size_of;
use std::sync::atomic::{AtomicU8, AtomicU16, AtomicU64, Ordering};

use cozy_chess::{Board, Move, Piece, Square};

use super::{MATE_THRESHOLD, Score};
use crate::engine::position::repetition_key;

pub(in crate::engine) const DEFAULT_HASH_MIB: usize = 16;
pub(in crate::engine) const MIN_HASH_MIB: usize = 1;
pub(in crate::engine) const MAX_HASH_MIB: usize = 1024;
const BUCKET_SIZE: usize = 4;
/// Halfmove clocks at or above this value keep individually keyed entries.
///
/// Below it every clock shares one class, so ordinary transpositions that differ
/// only in the fifty-move counter reuse each other's results. At or above it the
/// clock is keyed exactly, which keeps entries isolated for every position whose
/// score can depend on the rule-fifty horizon: static pruning stops here, null
/// pruning stops at ninety-nine, and a draw is claimed at one hundred.
pub(super) const RULE_FIFTY_EXACT_HORIZON: u8 = 80;
type Bucket = [Slot; BUCKET_SIZE];

/// Maps a halfmove clock onto the class that keys its transposition entries.
fn clock_class(halfmove_clock: u8) -> u8 {
    if halfmove_clock >= RULE_FIFTY_EXACT_HORIZON {
        halfmove_clock
    } else {
        0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Bound {
    Exact,
    Lower,
    Upper,
}

/// Marks an entry that carries no cached static evaluation.
///
/// Every real score is bounded by the mate constants, so this sits outside the
/// representable range rather than colliding with a legitimate evaluation.
const NO_STATIC_EVALUATION: i16 = i16::MIN;

/// Bits reserved for the aging generation inside a packed entry.
const GENERATION_BITS: u32 = 6;
/// Mask selecting a generation from a wider value.
const GENERATION_MASK: u8 = (1 << GENERATION_BITS) - 1;
/// Largest depth a packed entry can represent.
const MAX_STORED_DEPTH: u32 = u8::MAX as u32;

const MOVE_SHIFT: u32 = 0;
const SCORE_SHIFT: u32 = 16;
const STATIC_EVALUATION_SHIFT: u32 = 32;
const DEPTH_SHIFT: u32 = 48;
const GENERATION_SHIFT: u32 = 56;
const BOUND_SHIFT: u32 = 62;

/// Folds a clock class into a key so both are verified by one comparison.
///
/// The multiplier is odd, so distinct classes never cancel each other out, and
/// entries that differ only in their clock class fail verification rather than
/// being read across classes.
const CLOCK_CLASS_MULTIPLIER: u64 = 0x9e37_79b9_7f4a_7c15;

/// A stored search result.
///
/// The score and static evaluation are narrowed to sixteen bits, the depth to
/// eight, the generation to six, and the bound to two, which packs the whole
/// payload into one machine word. The position it belongs to is not part of the
/// payload: identity lives in the slot's verification word instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Entry {
    best_move: Option<Move>,
    score: i16,
    static_evaluation: i16,
    generation: u8,
    depth: u8,
    bound: Bound,
}

/// One concurrently readable table entry.
///
/// The payload occupies `data` and the position it describes is recorded as
/// `verify`, which holds the mixed key XORed with that payload. A reader that
/// observes two words from different writes recomputes a mismatched key and
/// rejects the slot, so a partially overwritten entry can be detected without
/// locking or reading both words in one operation. An unoccupied slot stores a
/// zero payload, which no real entry can produce because every bound is encoded
/// as a non-zero value.
#[derive(Debug, Default)]
struct Slot {
    verify: AtomicU64,
    data: AtomicU64,
}

/// A bucket decoded for a replacement decision.
type DecodedBucket = [Option<Entry>; BUCKET_SIZE];

const _: () = assert!(
    size_of::<Bucket>() == 64,
    "a bucket must occupy exactly one cache line",
);

/// Mixes a key and clock class into the identity a slot verifies against.
fn mixed_key(key: u64, halfmove_clock: u8) -> u64 {
    key ^ u64::from(clock_class(halfmove_clock)).wrapping_mul(CLOCK_CLASS_MULTIPLIER)
}

fn encode_move(chess_move: Option<Move>) -> u64 {
    let Some(chess_move) = chess_move else {
        return 0;
    };
    let promotion = chess_move.promotion.map_or(0, |piece| piece as u64 + 1);
    u64::from(chess_move.from as u8) | u64::from(chess_move.to as u8) << 6 | promotion << 12
}

/// Decodes a packed move, reporting `None` when no move was stored.
///
/// A stored move never has equal origin and destination squares, so an all-zero
/// move field cannot collide with a real one.
fn decode_move(bits: u64) -> Option<Move> {
    if bits == 0 {
        return None;
    }
    Some(Move {
        from: Square::index((bits & 0x3f) as usize),
        to: Square::index((bits >> 6 & 0x3f) as usize),
        promotion: match bits >> 12 & 0x7 {
            0 => None,
            encoded => Some(Piece::try_index(encoded as usize - 1)?),
        },
    })
}

const fn encode_bound(bound: Bound) -> u64 {
    match bound {
        Bound::Exact => 1,
        Bound::Lower => 2,
        Bound::Upper => 3,
    }
}

const fn decode_bound(bits: u64) -> Bound {
    match bits {
        1 => Bound::Exact,
        2 => Bound::Lower,
        _ => Bound::Upper,
    }
}

impl Slot {
    /// Returns the payload this slot holds and whether it belongs to `mixed`.
    ///
    /// One pair of loads answers both questions, so a replacement decision and
    /// an identity test never disagree about what the slot contained.
    fn snapshot(&self, mixed: u64) -> (Option<Entry>, bool) {
        let verify = self.verify.load(Ordering::Relaxed);
        let data = self.data.load(Ordering::Relaxed);
        (Entry::decode(data), verify ^ data == mixed && data != 0)
    }

    /// Returns the payload this slot holds for `mixed`, when it holds one.
    fn load_verified(&self, mixed: u64) -> Option<Entry> {
        let (entry, verified) = self.snapshot(mixed);
        verified.then_some(entry).flatten()
    }

    fn store(&self, mixed: u64, entry: Entry) {
        let data = entry.encode();
        self.verify.store(mixed ^ data, Ordering::Relaxed);
        self.data.store(data, Ordering::Relaxed);
    }

    fn clear(&self) {
        self.verify.store(0, Ordering::Relaxed);
        self.data.store(0, Ordering::Relaxed);
    }
}

impl Entry {
    /// Packs this entry into the single word a slot stores.
    fn encode(self) -> u64 {
        encode_move(self.best_move) << MOVE_SHIFT
            | u64::from(self.score as u16) << SCORE_SHIFT
            | u64::from(self.static_evaluation as u16) << STATIC_EVALUATION_SHIFT
            | u64::from(self.depth) << DEPTH_SHIFT
            | u64::from(self.generation & GENERATION_MASK) << GENERATION_SHIFT
            | encode_bound(self.bound) << BOUND_SHIFT
    }

    /// Unpacks a stored word, reporting `None` for an unoccupied slot.
    fn decode(data: u64) -> Option<Self> {
        if data == 0 {
            return None;
        }
        Some(Self {
            best_move: decode_move(data >> MOVE_SHIFT & 0xffff),
            score: (data >> SCORE_SHIFT) as u16 as i16,
            static_evaluation: (data >> STATIC_EVALUATION_SHIFT) as u16 as i16,
            depth: (data >> DEPTH_SHIFT) as u8,
            generation: (data >> GENERATION_SHIFT) as u8 & GENERATION_MASK,
            bound: decode_bound(data >> BOUND_SHIFT & 0x3),
        })
    }

    pub(super) fn depth(self) -> u32 {
        u32::from(self.depth)
    }

    pub(super) fn score_at_ply(self, ply: u32) -> Score {
        score_from_table(Score::from(self.score), ply)
    }

    /// Returns the static evaluation recorded with this entry, when it has one.
    pub(super) fn static_evaluation(self) -> Option<Score> {
        (self.static_evaluation != NO_STATIC_EVALUATION)
            .then_some(Score::from(self.static_evaluation))
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

/// Records that no evaluation profile has been observed yet.
///
/// Profiles are bounded well below this value, so it cannot collide with one.
const NO_EVALUATION_PROFILE: u16 = u16::MAX;

#[derive(Debug)]
pub(in crate::engine) struct TranspositionTable {
    buckets: Box<[Bucket]>,
    size_mib: usize,
    generation: AtomicU8,
    evaluation_profile: AtomicU16,
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
        buckets.extend((0..bucket_count).map(|_| Bucket::default()));

        Ok(Self {
            buckets: buckets.into_boxed_slice(),
            size_mib,
            generation: AtomicU8::new(0),
            evaluation_profile: AtomicU16::new(NO_EVALUATION_PROFILE),
        })
    }

    pub(in crate::engine) fn size_mib(&self) -> usize {
        self.size_mib
    }

    pub(super) fn start_search(&self, evaluation_profile: u8) {
        if self
            .evaluation_profile
            .swap(u16::from(evaluation_profile), Ordering::Relaxed)
            != u16::from(evaluation_profile)
        {
            self.discard_entries();
        }
        self.advance_generation();
    }

    pub(in crate::engine) fn clear(&self) {
        self.discard_entries();
        self.advance_generation();
    }

    pub(super) fn probe(&self, board: &Board) -> Option<Entry> {
        self.probe_key(repetition_key(board), board.halfmove_clock())
    }

    #[cfg(test)]
    pub(super) fn store(
        &self,
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
            None,
        );
    }
    pub(super) fn probe_key(&self, key: u64, halfmove_clock: u8) -> Option<Entry> {
        let mixed = mixed_key(key, halfmove_clock);
        self.buckets[self.index(key)]
            .iter()
            .find_map(|slot| slot.load_verified(mixed))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn store_key(
        &self,
        key: u64,
        halfmove_clock: u8,
        depth: u32,
        ply: u32,
        score: Score,
        bound: Bound,
        best_move: Option<Move>,
        static_evaluation: Option<Score>,
    ) {
        self.store_entry(
            key,
            mixed_key(key, halfmove_clock),
            Entry {
                depth: depth.min(MAX_STORED_DEPTH) as u8,
                score: narrow(score_to_table(score, ply)),
                static_evaluation: static_evaluation.map_or(NO_STATIC_EVALUATION, narrow),
                bound,
                best_move,
                generation: self.generation(),
            },
        );
    }

    /// Publishes an entry into the bucket its key selects.
    ///
    /// Every decision is taken from one snapshot of the bucket, so a concurrent
    /// writer can at worst cost this entry its slot or overwrite one this call
    /// chose to keep. Neither outcome can produce an unverifiable slot, because a
    /// slot is only ever written as a complete pair.
    fn store_entry(&self, key: u64, mixed: u64, candidate: Entry) {
        let generation = self.generation();
        let bucket = &self.buckets[self.index(key)];
        let mut decoded: DecodedBucket = [None; BUCKET_SIZE];
        let mut matching = None;
        for (index, slot) in bucket.iter().enumerate() {
            let (entry, verified) = slot.snapshot(mixed);
            decoded[index] = entry;
            if verified && matching.is_none() {
                matching = entry.map(|entry| (index, entry));
            }
        }

        if let Some((index, existing)) = matching {
            if existing.depth > candidate.depth && existing.bound == Bound::Exact {
                let mut refreshed = existing;
                refreshed.generation = generation;
                if refreshed.best_move.is_none() {
                    refreshed.best_move = candidate.best_move;
                }
                bucket[index].store(mixed, refreshed);
                return;
            }
            bucket[index].store(mixed, candidate);
            return;
        }

        if let Some(empty) = decoded.iter().position(Option::is_none) {
            bucket[empty].store(mixed, candidate);
            return;
        }

        let replacement = replacement_index(&decoded, generation);
        let existing = decoded[replacement].expect("a full bucket has a replacement entry");
        if existing.generation == generation
            && candidate.bound != Bound::Exact
            && (existing.bound == Bound::Exact || existing.depth > candidate.depth)
        {
            return;
        }
        bucket[replacement].store(mixed, candidate);
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

    fn generation(&self) -> u8 {
        self.generation.load(Ordering::Relaxed) & GENERATION_MASK
    }

    fn advance_generation(&self) {
        let next = self.generation().wrapping_add(1) & GENERATION_MASK;
        self.generation.store(next, Ordering::Relaxed);
    }

    fn discard_entries(&self) {
        for slot in self.buckets.iter().flatten() {
            slot.clear();
        }
    }

    fn index(&self, key: u64) -> usize {
        key as usize & (self.buckets.len() - 1)
    }
}

/// Selects the slot a new entry should take in a full bucket.
///
/// Entries from an earlier generation go first, oldest and least valuable
/// before newer ones. Generations wrap inside a narrow field, so age is
/// measured as a modular distance rather than a difference.
fn replacement_index(bucket: &DecodedBucket, generation: u8) -> usize {
    if let Some((index, _)) = bucket
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| entry.map(|entry| (index, entry)))
        .filter(|(_, entry)| entry.generation != generation)
        .max_by_key(|(_, entry)| {
            (
                generation.wrapping_sub(entry.generation) & GENERATION_MASK,
                entry.bound != Bound::Exact,
                u32::MAX - entry.depth(),
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

/// Narrows a search score to the width stored in a table entry.
///
/// Every score search can produce lies inside the mate bounds, so this clamp is
/// unreachable in practice and exists to keep the conversion total.
fn narrow(score: Score) -> i16 {
    score.clamp(i16::MIN as Score + 1, i16::MAX as Score) as i16
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
    use std::sync::atomic::Ordering;

    use super::{Bound, MATE_THRESHOLD, TranspositionTable};
    use crate::engine::Position;

    #[test]
    fn stores_and_recovers_entries() {
        let position = Position::default();
        let best_move = position.search_moves()[0];
        let table = TranspositionTable::new(1).unwrap();

        table.store(position.board(), 6, 0, 42, Bound::Exact, Some(best_move));
        let entry = table.probe(position.board()).unwrap();

        assert_eq!(entry.depth(), 6);
        assert_eq!(entry.score_at_ply(0), 42);
        assert_eq!(entry.bound(), Bound::Exact);
        assert_eq!(entry.best_move(), Some(best_move));
    }

    /// Every field must survive the round trip through one packed word.
    #[test]
    fn packing_preserves_every_stored_field() {
        use cozy_chess::{Move, Piece, Square};

        let promotion = Move {
            from: Square::B7,
            to: Square::A8,
            promotion: Some(Piece::Knight),
        };
        for best_move in [None, Some(promotion)] {
            for bound in [Bound::Exact, Bound::Lower, Bound::Upper] {
                for (score, static_evaluation) in [
                    (0, None),
                    (-4_321, Some(1_234)),
                    (i16::MAX.into(), Some(-1)),
                ] {
                    let entry = super::Entry {
                        best_move,
                        score: score as i16,
                        static_evaluation: static_evaluation
                            .map_or(super::NO_STATIC_EVALUATION, |value: i32| value as i16),
                        generation: 5,
                        depth: 200,
                        bound,
                    };

                    assert_eq!(
                        super::Entry::decode(entry.encode()),
                        Some(entry),
                        "{entry:?} did not survive packing",
                    );
                }
            }
        }
    }

    /// A stored payload must never decode as an unoccupied slot.
    ///
    /// Emptiness is represented by an all-zero word, so a real entry that
    /// happened to encode to zero would silently vanish.
    #[test]
    fn no_stored_entry_encodes_as_empty() {
        let entry = super::Entry {
            best_move: None,
            score: 0,
            static_evaluation: 0,
            generation: 0,
            depth: 0,
            bound: Bound::Exact,
        };

        assert_ne!(entry.encode(), 0);
        assert_eq!(super::Entry::decode(0), None);
    }

    /// Clock classes must be distinguished by the verification word alone.
    #[test]
    fn mixed_keys_separate_clock_classes() {
        let shared = super::mixed_key(7, 0);

        assert_eq!(
            shared,
            super::mixed_key(7, super::RULE_FIFTY_EXACT_HORIZON - 1)
        );
        assert_ne!(shared, super::mixed_key(7, super::RULE_FIFTY_EXACT_HORIZON));
        assert_ne!(
            super::mixed_key(7, super::RULE_FIFTY_EXACT_HORIZON),
            super::mixed_key(7, super::RULE_FIFTY_EXACT_HORIZON + 1),
        );
    }

    /// A torn slot must be rejected rather than read as a valid entry.
    #[test]
    fn a_half_written_slot_does_not_verify() {
        let position = Position::default();
        let table = TranspositionTable::new(1).unwrap();
        let key = super::repetition_key(position.board());
        table.store(position.board(), 6, 0, 42, Bound::Exact, None);
        assert!(table.probe(position.board()).is_some());

        // Simulate a writer that published a payload for a different position
        // without its matching verification word.
        let bucket = &table.buckets[table.index(key)];
        let slot = bucket
            .iter()
            .find(|slot| slot.load_verified(super::mixed_key(key, 0)).is_some())
            .expect("the stored entry occupies a slot");
        slot.data
            .store(slot.data.load(Ordering::Relaxed) ^ 1, Ordering::Relaxed);

        assert!(
            table.probe(position.board()).is_none(),
            "a mismatched payload must not be reported as a hit",
        );
    }

    #[test]
    fn stored_static_evaluations_are_recovered_and_optional() {
        let position = Position::default();
        let best_move = position.search_moves()[0];
        let table = TranspositionTable::new(1).unwrap();

        table.store_key(
            super::repetition_key(position.board()),
            position.board().halfmove_clock(),
            6,
            0,
            42,
            Bound::Exact,
            Some(best_move),
            Some(-37),
        );
        assert_eq!(
            table.probe(position.board()).unwrap().static_evaluation(),
            Some(-37),
        );

        table.clear();
        table.store(position.board(), 6, 0, 42, Bound::Exact, Some(best_move));

        assert_eq!(
            table.probe(position.board()).unwrap().static_evaluation(),
            None,
            "an entry stored without an evaluation must not invent one",
        );
    }

    fn synthetic_entry(depth: u32, bound: Bound, generation: u8) -> super::Entry {
        super::Entry {
            depth: depth as u8,
            score: 0,
            static_evaluation: super::NO_STATIC_EVALUATION,
            bound,
            best_move: None,
            generation,
        }
    }

    /// Stores a synthetic entry under `key`, bypassing score normalization.
    fn store_synthetic(table: &TranspositionTable, key: u64, entry: super::Entry) {
        table.store_entry(key, super::mixed_key(key, 0), entry);
    }

    #[test]
    fn colliding_entries_share_a_bucket() {
        let table = TranspositionTable::new(1).unwrap();
        table.start_search(0);
        let stride = table.buckets.len() as u64;

        for slot in 0..super::BUCKET_SIZE {
            store_synthetic(
                &table,
                slot as u64 * stride,
                synthetic_entry(slot as u32 + 1, Bound::Lower, table.generation()),
            );
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
        let table = TranspositionTable::new(1).unwrap();
        table.start_search(0);
        let stride = table.buckets.len() as u64;
        let entries = [
            (0, 8, Bound::Exact),
            (1, 6, Bound::Exact),
            (2, 4, Bound::Lower),
            (3, 2, Bound::Upper),
        ];
        for (slot, depth, bound) in entries {
            store_synthetic(
                &table,
                slot * stride,
                synthetic_entry(depth, bound, table.generation()),
            );
        }

        store_synthetic(
            &table,
            4 * stride,
            synthetic_entry(3, Bound::Lower, table.generation()),
        );

        assert!(table.probe_key(3 * stride, 0).is_none());
        assert!(table.probe_key(4 * stride, 0).is_some());
        assert_eq!(table.probe_key(0, 0).unwrap().depth(), 8);
    }

    #[test]
    fn stale_entries_yield_to_the_current_generation() {
        let table = TranspositionTable::new(1).unwrap();
        table.start_search(0);
        let stride = table.buckets.len() as u64;
        for slot in 0..super::BUCKET_SIZE {
            store_synthetic(
                &table,
                slot as u64 * stride,
                synthetic_entry(8, Bound::Exact, table.generation()),
            );
        }

        table.start_search(0);
        store_synthetic(
            &table,
            4 * stride,
            synthetic_entry(1, Bound::Upper, table.generation()),
        );

        assert!(table.probe_key(4 * stride, 0).is_some());
    }

    /// Aging must keep working when the narrow generation field wraps.
    ///
    /// With generation two current, generation `GENERATION_MASK` sits three
    /// generations back rather than sixty-one ahead, so it is the oldest entry
    /// present and must be replaced before the more recent ones.
    #[test]
    fn replacement_measures_age_across_a_generation_wrap() {
        let bucket = [
            Some(synthetic_entry(8, Bound::Exact, 1)),
            Some(synthetic_entry(8, Bound::Exact, super::GENERATION_MASK)),
            Some(synthetic_entry(8, Bound::Exact, 0)),
            Some(synthetic_entry(8, Bound::Exact, 2)),
        ];

        assert_eq!(
            super::replacement_index(&bucket, 2),
            1,
            "the entry from before the wrap is the oldest",
        );
    }

    #[test]
    fn a_deeper_matching_exact_entry_is_preserved() {
        let table = TranspositionTable::new(1).unwrap();
        table.start_search(0);
        let key = 7;
        store_synthetic(
            &table,
            key,
            synthetic_entry(8, Bound::Exact, table.generation()),
        );
        store_synthetic(
            &table,
            key,
            synthetic_entry(2, Bound::Lower, table.generation()),
        );

        let entry = table.probe_key(key, 0).unwrap();
        assert_eq!(entry.depth(), 8);
        assert_eq!(entry.bound(), Bound::Exact);
        assert_eq!(entry.generation, table.generation());
    }

    #[test]
    fn an_equal_depth_result_replaces_a_matching_exact_entry() {
        let table = TranspositionTable::new(1).unwrap();
        table.start_search(0);
        let key = 11;
        store_synthetic(
            &table,
            key,
            synthetic_entry(5, Bound::Exact, table.generation()),
        );
        let mut refreshed = synthetic_entry(5, Bound::Lower, table.generation());
        refreshed.score = 64;
        store_synthetic(&table, key, refreshed);

        let entry = table.probe_key(key, 0).unwrap();
        assert_eq!(entry.depth(), 5);
        assert_eq!(entry.bound(), Bound::Lower);
        assert_eq!(entry.score_at_ply(0), 64);
    }

    #[test]
    fn shares_entries_below_the_rule_fifty_horizon_and_isolates_above_it() {
        let quiet = Position::from_fen("7k/8/8/8/8/8/R7/K7 w - - 0 1").unwrap();
        let advanced = Position::from_fen("7k/8/8/8/8/8/R7/K7 w - - 40 21").unwrap();
        let horizon = Position::from_fen("7k/8/8/8/8/8/R7/K7 w - - 80 41").unwrap();
        let claimable = Position::from_fen("7k/8/8/8/8/8/R7/K7 w - - 99 50").unwrap();
        let table = TranspositionTable::new(1).unwrap();

        table.store(quiet.board(), 4, 0, 75, Bound::Exact, None);

        assert_eq!(
            table
                .probe(advanced.board())
                .map(|entry| entry.score_at_ply(0)),
            Some(75),
            "clocks below the horizon must share one entry",
        );
        assert!(
            table.probe(horizon.board()).is_none(),
            "the horizon clock must not read a shared entry",
        );
        assert!(
            table.probe(claimable.board()).is_none(),
            "a nearly claimable draw must not read a shared entry",
        );

        table.store(claimable.board(), 4, 0, 120, Bound::Exact, None);

        assert_eq!(
            table
                .probe(claimable.board())
                .map(|entry| entry.score_at_ply(0)),
            Some(120),
        );
        assert!(
            table.probe(horizon.board()).is_none(),
            "clocks at or above the horizon stay individually keyed",
        );
        assert_eq!(
            table
                .probe(quiet.board())
                .map(|entry| entry.score_at_ply(0)),
            Some(75),
            "isolating a claimable clock must not disturb the shared class",
        );
    }

    #[test]
    fn normalizes_mate_scores_across_root_plies() {
        let position = Position::default();
        let table = TranspositionTable::new(1).unwrap();

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
        let table = TranspositionTable::new(1).unwrap();
        table.store(position.board(), 1, 0, 0, Bound::Upper, None);

        table.clear();

        assert!(table.probe(position.board()).is_none());
    }
    #[test]
    fn evaluation_profile_changes_discard_entries() {
        let position = Position::default();
        let table = TranspositionTable::new(1).unwrap();
        table.start_search(0);
        table.store(position.board(), 1, 0, 0, Bound::Upper, None);

        table.start_search(0);
        assert!(table.probe(position.board()).is_some());

        table.start_search(100);
        assert!(table.probe(position.board()).is_none());
    }

    /// Concurrent writers must never publish a slot that verifies incorrectly.
    ///
    /// Many threads hammer one bucket with distinct keys, each carrying a depth
    /// that identifies its key. Every successful probe must decode to the depth
    /// that key was stored with, which is what a torn read would violate.
    #[test]
    fn concurrent_writers_never_publish_a_torn_entry() {
        use std::sync::Arc;

        const WRITERS: u64 = 8;
        const ROUNDS: u64 = 4_000;

        let table = Arc::new(TranspositionTable::new(1).unwrap());
        table.start_search(0);
        let stride = table.buckets.len() as u64;
        let writers = (1..=WRITERS)
            .map(|writer| {
                let table = Arc::clone(&table);
                std::thread::spawn(move || {
                    for _ in 0..ROUNDS {
                        let key = writer * stride;
                        table.store_key(
                            key,
                            0,
                            writer as u32,
                            0,
                            writer as super::Score,
                            Bound::Exact,
                            None,
                            None,
                        );
                        for probe in 1..=WRITERS {
                            if let Some(entry) = table.probe_key(probe * stride, 0) {
                                assert_eq!(
                                    entry.depth(),
                                    probe as u32,
                                    "key {probe} returned another key's payload",
                                );
                                assert_eq!(entry.score_at_ply(0), probe as super::Score);
                            }
                        }
                    }
                })
            })
            .collect::<Vec<_>>();

        for writer in writers {
            writer.join().unwrap();
        }
    }
}
