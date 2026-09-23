use std::fmt::{self, Display, Formatter};
use std::mem::{align_of, size_of};
use std::sync::atomic::{AtomicU8, AtomicU16, AtomicU64, Ordering};

use cozy_chess::{Board, Move, Piece, Square};

use super::{MATE_THRESHOLD, Score};
use crate::engine::position::repetition_key;

mod memory;

use memory::BucketMemory;

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
/// The slots one key selects, pinned to a cache line.
///
/// The alignment is part of the design rather than a property of whichever
/// allocator happens to be linked: a probe or store touches exactly one line,
/// and a prefetch of the bucket's address brings in every slot.
#[derive(Debug, Default)]
#[repr(align(64))]
struct Bucket([Slot; BUCKET_SIZE]);

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

/// The payload words of one bucket, read once for a replacement decision.
type BucketWords = [u64; BUCKET_SIZE];

/// What a probe read from the bucket its key selects.
///
/// The words are the snapshot a store of the same position decides its
/// replacement from, so a node's store issues no loads: the probe at node entry
/// has already read the bucket. On a hit the scan stops at the matching slot, so
/// only that word is meaningful; on a miss every slot was read.
#[derive(Clone, Copy, Debug)]
pub(super) struct Probe {
    key: u64,
    mixed: u64,
    words: BucketWords,
    /// The slot that held this position when the bucket was read.
    matching: Option<u8>,
    entry: Option<Entry>,
}

impl Probe {
    /// Returns the stored result for the probed position, when one was found.
    #[inline(always)]
    pub(super) fn entry(&self) -> Option<Entry> {
        self.entry
    }
}

/// Selects the move field of a packed word.
const MOVE_FIELD: u64 = 0xffff << MOVE_SHIFT;
/// Selects the generation field of a packed word.
const GENERATION_FIELD: u64 = (GENERATION_MASK as u64) << GENERATION_SHIFT;

const _: () = assert!(
    size_of::<Bucket>() == 64 && align_of::<Bucket>() == 64,
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
#[inline(always)]
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

/// Reads the depth field of a packed word.
#[inline(always)]
fn packed_depth(data: u64) -> u8 {
    (data >> DEPTH_SHIFT) as u8
}

/// Reads the generation field of a packed word.
#[inline(always)]
fn packed_generation(data: u64) -> u8 {
    (data >> GENERATION_SHIFT) as u8 & GENERATION_MASK
}

/// Reports whether a packed word carries an exact bound.
#[inline(always)]
fn packed_is_exact(data: u64) -> bool {
    data >> BOUND_SHIFT == encode_bound(Bound::Exact)
}

impl Slot {
    /// Returns the verification and payload words this slot holds.
    ///
    /// One pair of loads answers every question a probe or store asks, so a
    /// replacement decision and an identity test never disagree about what the
    /// slot held.
    #[inline(always)]
    fn load(&self) -> (u64, u64) {
        (
            self.verify.load(Ordering::Relaxed),
            self.data.load(Ordering::Relaxed),
        )
    }

    fn store(&self, mixed: u64, data: u64) {
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
    #[inline(always)]
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
    memory: BucketMemory,
    size_mib: usize,
    generation: AtomicU8,
    evaluation_profile: AtomicU16,
}

// Searchers share one table through an `Arc`, so it must stay shareable when
// its memory is described by a raw pointer rather than a box.
const _: () = {
    const fn shareable<T: Send + Sync>() {}
    shareable::<TranspositionTable>();
};

impl TranspositionTable {
    /// Allocates a table of as many buckets as fit in the requested mebibytes.
    ///
    /// A bucket count need not be a power of two, so the table uses the memory
    /// it was given rather than the power of two below it.
    pub(in crate::engine) fn new(size_mib: usize) -> Result<Self, AllocationError> {
        let bytes = size_mib.checked_mul(1024 * 1024).ok_or(AllocationError)?;
        let bucket_count = bytes / size_of::<Bucket>();
        if bucket_count == 0 {
            return Err(AllocationError);
        }

        Ok(Self {
            memory: BucketMemory::zeroed(bucket_count)?,
            size_mib,
            generation: AtomicU8::new(0),
            evaluation_profile: AtomicU16::new(NO_EVALUATION_PROFILE),
        })
    }

    pub(in crate::engine) fn size_mib(&self) -> usize {
        self.size_mib
    }

    #[inline(always)]
    fn buckets(&self) -> &[Bucket] {
        self.memory.buckets()
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
            .entry()
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

    /// Starts pulling the bucket a key selects into cache.
    ///
    /// A probe is one dependent load from a table far larger than any cache,
    /// so it stalls for a memory round trip unless the line is already on its
    /// way. Issuing the hint as soon as a child's key is known lets the move
    /// bookkeeping and node entry overlap that latency. A hint never faults and
    /// changes no state, so it is safe to issue for a key that is never probed.
    #[inline(always)]
    pub(super) fn prefetch(&self, key: u64) {
        #[cfg(target_arch = "x86_64")]
        {
            use std::arch::x86_64::{_MM_HINT_T0, _mm_prefetch};
            let bucket: *const Bucket = &self.buckets()[self.index(key)];
            // SAFETY: SSE is part of the x86_64 baseline and the pointer lies
            // inside the table; a prefetch reads nothing and cannot fault.
            unsafe { _mm_prefetch(bucket.cast::<i8>(), _MM_HINT_T0) };
        }
        #[cfg(target_arch = "aarch64")]
        {
            let bucket: *const Bucket = &self.buckets()[self.index(key)];
            // SAFETY: `prfm` is a hint that reads nothing, writes nothing and
            // cannot fault, whatever the address.
            unsafe {
                std::arch::asm!(
                    "prfm pldl1keep, [{bucket}]",
                    bucket = in(reg) bucket,
                    options(nostack, preserves_flags, readonly),
                );
            }
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        let _ = key;
    }
    /// Reads the bucket a key selects, stopping at the slot that holds it.
    ///
    /// What was read travels with the result, so the store this node ends with
    /// decides its replacement without reading the bucket again.
    #[inline(always)]
    pub(super) fn probe_key(&self, key: u64, halfmove_clock: u8) -> Probe {
        let mixed = mixed_key(key, halfmove_clock);
        let bucket = &self.buckets()[self.index(key)];
        let mut words: BucketWords = [0; BUCKET_SIZE];
        for (index, slot) in bucket.0.iter().enumerate() {
            let (verify, data) = slot.load();
            words[index] = data;
            if data != 0 && verify ^ data == mixed {
                return Probe {
                    key,
                    mixed,
                    words,
                    matching: Some(index as u8),
                    entry: Entry::decode(data),
                };
            }
        }
        Probe {
            key,
            mixed,
            words,
            matching: None,
            entry: None,
        }
    }

    /// Probes a position and stores a result for it, as a search node does.
    #[cfg(test)]
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
        self.store_probed(
            &self.probe_key(key, halfmove_clock),
            depth,
            ply,
            score,
            bound,
            best_move,
            static_evaluation,
        );
    }

    /// Publishes a result for the position a probe looked up.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn store_probed(
        &self,
        probe: &Probe,
        depth: u32,
        ply: u32,
        score: Score,
        bound: Bound,
        best_move: Option<Move>,
        static_evaluation: Option<Score>,
    ) {
        self.store_entry(
            probe,
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

    /// Publishes an entry into the bucket a probe read.
    ///
    /// Every decision is taken from the words the probe read rather than from a
    /// fresh read of the bucket, so a store issues no loads and at most two
    /// stores. That snapshot may be stale: this searcher's own subtree, or
    /// another searcher, may have written the bucket since. A stale decision can
    /// cost this entry its slot, overwrite one a fresh read would have kept, or
    /// restore an older result of the same position, which is exactly what a
    /// lost race between two writers already costs. It cannot produce an
    /// unverifiable slot, because a slot is only ever written as a complete pair.
    fn store_entry(&self, probe: &Probe, candidate: Entry) {
        let Some((slot, data)) = placement(probe, candidate, self.generation()) else {
            return;
        };
        self.buckets()[self.index(probe.key)].0[slot].store(probe.mixed, data);
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
        for slot in self.buckets().iter().flat_map(|bucket| &bucket.0) {
            slot.clear();
        }
    }

    /// Selects the bucket a key addresses.
    ///
    /// The high word of the key times the bucket count lies in `0..count` for
    /// any count, so the table is not bound to a power of two. Keys that share
    /// a bucket therefore agree in their high bits, and the verification word
    /// tells them apart as it did keys agreeing in their low bits under a mask.
    #[inline(always)]
    fn index(&self, key: u64) -> usize {
        ((u128::from(key) * self.buckets().len() as u128) >> 64) as usize
    }
}

/// How much shallower than the entry it finds an inexact store of the same
/// position must be for the entry to be kept instead.
///
/// A position reached again through a reduction, or at the horizon by
/// quiescence, is searched far shallower than when it was first stored, and
/// its bound says less than the one it would replace.
const SAME_POSITION_DEPTH_MARGIN: u8 = 4;

/// Decides what a store writes, from the words a probe read.
///
/// Returns the slot to write and the word to write there, or `None` when the
/// bucket is left as it is. The slot the probe found the position in is
/// refreshed when it holds a deeper exact result, or an inexact candidate is
/// at least [`SAME_POSITION_DEPTH_MARGIN`] plies shallower than it, and
/// overwritten otherwise; a candidate without a move keeps the move the slot
/// recorded. A position the probe missed takes an empty slot, or else the slot
/// `replacement_index` selects unless that holds a current-generation result
/// the candidate cannot improve on. Only the depth, generation, bound and move
/// fields of the packed words are read; nothing is decoded, and a slot that
/// already holds this exact payload is left untouched.
fn placement(probe: &Probe, candidate: Entry, generation: u8) -> Option<(usize, u64)> {
    let data = candidate.encode();
    if let Some(index) = probe.matching.map(usize::from) {
        let existing = probe.words[index];
        if existing == data {
            return None;
        }
        let existing_depth = packed_depth(existing);
        let keeps_existing = (existing_depth > candidate.depth && packed_is_exact(existing))
            || (candidate.bound != Bound::Exact
                && existing_depth >= candidate.depth.saturating_add(SAME_POSITION_DEPTH_MARGIN));
        if keeps_existing {
            let mut refreshed =
                existing & !GENERATION_FIELD | u64::from(generation) << GENERATION_SHIFT;
            if existing & MOVE_FIELD == 0 {
                refreshed |= data & MOVE_FIELD;
            }
            return (refreshed != existing).then_some((index, refreshed));
        }
        if data & MOVE_FIELD == 0 {
            return Some((index, data | existing & MOVE_FIELD));
        }
        return Some((index, data));
    }

    if let Some(empty) = probe.words.iter().position(|&word| word == 0) {
        return Some((empty, data));
    }

    let replacement = replacement_index(&probe.words, generation);
    let existing = probe.words[replacement];
    if packed_generation(existing) == generation
        && candidate.bound != Bound::Exact
        && (packed_is_exact(existing) || packed_depth(existing) > candidate.depth)
    {
        return None;
    }
    Some((replacement, data))
}

/// Selects the slot a new entry should take in a full bucket.
///
/// Entries from an earlier generation go first, oldest and least valuable
/// before newer ones; among equals the last wins. When every entry is current,
/// the shallowest inexact entry goes, then the shallowest exact one; among
/// equals the first wins. Generations wrap inside a narrow field, so age is
/// measured as a modular distance rather than a difference.
///
/// Both orderings are folded into one integer per word so the whole decision
/// is a handful of shifts and compares over the packed bits.
fn replacement_index(bucket: &BucketWords, generation: u8) -> usize {
    let mut stale: Option<(usize, u32)> = None;
    let mut current: Option<(usize, u32)> = None;
    for (index, &data) in bucket.iter().enumerate() {
        let depth = u32::from(packed_depth(data));
        let exact = packed_is_exact(data);
        let age = u32::from(generation.wrapping_sub(packed_generation(data)) & GENERATION_MASK);
        if age != 0 {
            let rank = age << 9 | u32::from(!exact) << 8 | (u32::from(u8::MAX) - depth);
            if stale.is_none_or(|(_, best)| rank >= best) {
                stale = Some((index, rank));
            }
        } else {
            let rank = u32::from(exact) << 8 | depth;
            if current.is_none_or(|(_, best)| rank < best) {
                current = Some((index, rank));
            }
        }
    }
    stale
        .or(current)
        .map(|(index, _)| index)
        .expect("a full bucket has an occupied entry")
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
        let bucket = &table.buckets()[table.index(key)];
        let slot = table
            .probe_key(key, 0)
            .matching
            .map(|index| &bucket.0[usize::from(index)])
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
        table.store_entry(&table.probe_key(key, 0), entry);
    }

    /// Returns the `ordinal`-th key that selects the table's first bucket.
    ///
    /// A bucket is chosen by the high word of the key times the bucket count,
    /// so every key below `u64::MAX / count` selects bucket zero: distinct small
    /// integers collide.
    fn colliding_key(table: &TranspositionTable, ordinal: u64) -> u64 {
        assert_eq!(table.index(ordinal), 0, "key {ordinal} misses bucket zero");
        ordinal
    }

    #[test]
    fn colliding_entries_share_a_bucket() {
        let table = TranspositionTable::new(1).unwrap();
        table.start_search(0);

        for slot in 0..super::BUCKET_SIZE {
            store_synthetic(
                &table,
                colliding_key(&table, slot as u64),
                synthetic_entry(slot as u32 + 1, Bound::Lower, table.generation()),
            );
        }

        for slot in 0..super::BUCKET_SIZE {
            assert!(
                table
                    .probe_key(colliding_key(&table, slot as u64), 0)
                    .entry()
                    .is_some()
            );
        }
    }

    /// The table uses every mebibyte it is given, not the power of two below.
    #[test]
    fn bucket_allocation_uses_the_requested_size() {
        for size_mib in [1, 3] {
            let table = TranspositionTable::new(size_mib).unwrap();
            let bytes = table.buckets().len() * std::mem::size_of::<super::Bucket>();

            assert_eq!(bytes, size_mib * 1024 * 1024);
        }
        assert!(
            !TranspositionTable::new(3)
                .unwrap()
                .buckets()
                .len()
                .is_power_of_two()
        );
    }

    /// Every key must land inside a bucket count that is not a power of two,
    /// and the keys must spread across the whole table.
    #[test]
    fn indexing_covers_any_bucket_count() {
        let table = TranspositionTable::new(3).unwrap();
        let count = table.buckets().len();

        assert_eq!(table.index(0), 0);
        assert_eq!(table.index(u64::MAX), count - 1);
        assert_eq!(table.index(1 << 63), count / 2);
        let mut key = 0x9e37_79b9_7f4a_7c15_u64;
        let mut touched = vec![false; count];
        for _ in 0..(count * 16) {
            key = key.wrapping_mul(0x5851_f42d_4c95_7f2d).wrapping_add(1);
            touched[table.index(key)] = true;
        }
        let coverage = touched.iter().filter(|&&hit| hit).count();
        assert!(
            coverage * 100 >= count * 99,
            "{coverage} of {count} buckets"
        );
    }

    #[test]
    fn replacement_prefers_the_shallowest_non_exact_entry() {
        let table = TranspositionTable::new(1).unwrap();
        table.start_search(0);
        let entries = [
            (0, 8, Bound::Exact),
            (1, 6, Bound::Exact),
            (2, 4, Bound::Lower),
            (3, 2, Bound::Upper),
        ];
        for (slot, depth, bound) in entries {
            store_synthetic(
                &table,
                colliding_key(&table, slot),
                synthetic_entry(depth, bound, table.generation()),
            );
        }

        store_synthetic(
            &table,
            colliding_key(&table, 4),
            synthetic_entry(3, Bound::Lower, table.generation()),
        );

        assert!(
            table
                .probe_key(colliding_key(&table, 3), 0)
                .entry()
                .is_none()
        );
        assert!(
            table
                .probe_key(colliding_key(&table, 4), 0)
                .entry()
                .is_some()
        );
        assert_eq!(table.probe_key(0, 0).entry().unwrap().depth(), 8);
    }

    #[test]
    fn stale_entries_yield_to_the_current_generation() {
        let table = TranspositionTable::new(1).unwrap();
        table.start_search(0);
        for slot in 0..super::BUCKET_SIZE {
            store_synthetic(
                &table,
                colliding_key(&table, slot as u64),
                synthetic_entry(8, Bound::Exact, table.generation()),
            );
        }

        table.start_search(0);
        store_synthetic(
            &table,
            colliding_key(&table, 4),
            synthetic_entry(1, Bound::Upper, table.generation()),
        );

        assert!(
            table
                .probe_key(colliding_key(&table, 4), 0)
                .entry()
                .is_some()
        );
    }

    /// Aging must keep working when the narrow generation field wraps.
    ///
    /// With generation two current, generation `GENERATION_MASK` sits three
    /// generations back rather than sixty-one ahead, so it is the oldest entry
    /// present and must be replaced before the more recent ones.
    #[test]
    fn replacement_measures_age_across_a_generation_wrap() {
        let bucket = [
            synthetic_entry(8, Bound::Exact, 1).encode(),
            synthetic_entry(8, Bound::Exact, super::GENERATION_MASK).encode(),
            synthetic_entry(8, Bound::Exact, 0).encode(),
            synthetic_entry(8, Bound::Exact, 2).encode(),
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

        let entry = table.probe_key(key, 0).entry().unwrap();
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

        let entry = table.probe_key(key, 0).entry().unwrap();
        assert_eq!(entry.depth(), 5);
        assert_eq!(entry.bound(), Bound::Lower);
        assert_eq!(entry.score_at_ply(0), 64);
    }

    /// A much shallower inexact result of the same position, such as one
    /// quiescence settles at the horizon, keeps the deeper entry.
    #[test]
    fn a_much_shallower_inexact_store_keeps_the_deeper_entry() {
        let table = TranspositionTable::new(1).unwrap();
        table.start_search(0);
        let key = 13;
        store_synthetic(&table, key, synthetic_entry(8, Bound::Lower, 0));
        store_synthetic(
            &table,
            key,
            synthetic_entry(
                8 - u32::from(super::SAME_POSITION_DEPTH_MARGIN),
                Bound::Upper,
                1,
            ),
        );

        let entry = table.probe_key(key, 0).entry().unwrap();
        assert_eq!(entry.depth(), 8);
        assert_eq!(entry.bound(), Bound::Lower);
        assert_eq!(entry.generation, table.generation());

        store_synthetic(
            &table,
            key,
            synthetic_entry(
                9 - u32::from(super::SAME_POSITION_DEPTH_MARGIN),
                Bound::Upper,
                table.generation(),
            ),
        );
        let entry = table.probe_key(key, 0).entry().unwrap();
        assert_eq!(
            entry.depth(),
            9 - u32::from(super::SAME_POSITION_DEPTH_MARGIN),
            "a result inside the margin replaces the entry",
        );
        assert_eq!(entry.bound(), Bound::Upper);
    }

    #[test]
    fn a_shallower_exact_store_replaces_a_deeper_inexact_entry() {
        let table = TranspositionTable::new(1).unwrap();
        table.start_search(0);
        let key = 17;
        store_synthetic(
            &table,
            key,
            synthetic_entry(10, Bound::Lower, table.generation()),
        );
        store_synthetic(
            &table,
            key,
            synthetic_entry(0, Bound::Exact, table.generation()),
        );

        let entry = table.probe_key(key, 0).entry().unwrap();
        assert_eq!(entry.depth(), 0);
        assert_eq!(entry.bound(), Bound::Exact);
    }

    /// A result that names no move keeps the one the position already has,
    /// whether it replaces the entry or not.
    #[test]
    fn a_store_without_a_move_keeps_the_recorded_move() {
        let position = Position::default();
        let best_move = position.search_moves()[0];
        let table = TranspositionTable::new(1).unwrap();
        table.start_search(0);

        table.store(position.board(), 3, 0, 20, Bound::Lower, Some(best_move));
        table.store(position.board(), 3, 0, -5, Bound::Upper, None);
        let replaced = table.probe(position.board()).unwrap();
        assert_eq!(replaced.bound(), Bound::Upper);
        assert_eq!(replaced.score_at_ply(0), -5);
        assert_eq!(replaced.best_move(), Some(best_move));

        table.store(position.board(), 12, 0, 30, Bound::Lower, Some(best_move));
        table.store(position.board(), 0, 0, 10, Bound::Upper, None);
        let kept = table.probe(position.board()).unwrap();
        assert_eq!(kept.depth(), 12);
        assert_eq!(kept.best_move(), Some(best_move));
    }

    /// A store decided from a stale probe publishes only its own position.
    ///
    /// The bucket is filled by other positions between the probe and the
    /// store, so the empty slot the probe saw is taken. The store still lands
    /// as a complete pair: its own position reads back, the position it
    /// displaced reads as nothing, and no key ever reads another's payload.
    #[test]
    fn a_stale_probe_still_publishes_a_verifiable_entry() {
        let table = TranspositionTable::new(1).unwrap();
        table.start_search(0);
        let stale = table.probe_key(0, 0);
        assert!(stale.entry().is_none());

        for slot in 1..=super::BUCKET_SIZE as u64 {
            store_synthetic(
                &table,
                colliding_key(&table, slot),
                synthetic_entry(slot as u32, Bound::Exact, table.generation()),
            );
        }

        table.store_entry(&stale, synthetic_entry(9, Bound::Exact, table.generation()));

        let entry = table
            .probe_key(0, 0)
            .entry()
            .expect("the stale store publishes its own position");
        assert_eq!(entry.depth(), 9);
        let survivors = (1..=super::BUCKET_SIZE as u64)
            .filter(|&slot| {
                table
                    .probe_key(colliding_key(&table, slot), 0)
                    .entry()
                    .inspect(|entry| {
                        assert_eq!(
                            entry.depth(),
                            slot as u32,
                            "key {slot} read another payload"
                        );
                    })
                    .is_some()
            })
            .count();
        assert_eq!(
            survivors,
            super::BUCKET_SIZE - 1,
            "a stale decision costs exactly the slot it took",
        );
    }

    /// A stale hit writes the probed position back over whatever took its slot.
    ///
    /// The displaced position reads as nothing rather than as the payload of
    /// the position that reclaimed the slot.
    #[test]
    fn a_stale_hit_restores_only_the_probed_position() {
        let table = TranspositionTable::new(1).unwrap();
        table.start_search(0);
        store_synthetic(
            &table,
            0,
            synthetic_entry(1, Bound::Lower, table.generation()),
        );
        for slot in 1..super::BUCKET_SIZE as u64 {
            store_synthetic(
                &table,
                colliding_key(&table, slot),
                synthetic_entry(8, Bound::Exact, table.generation()),
            );
        }
        let stale = table.probe_key(0, 0);
        assert_eq!(stale.entry().map(|entry| entry.depth()), Some(1));

        // The shallow inexact entry is the one a new position replaces.
        let intruder = colliding_key(&table, super::BUCKET_SIZE as u64);
        store_synthetic(
            &table,
            intruder,
            synthetic_entry(5, Bound::Exact, table.generation()),
        );
        assert!(table.probe_key(0, 0).entry().is_none());
        assert!(table.probe_key(intruder, 0).entry().is_some());

        table.store_entry(&stale, synthetic_entry(2, Bound::Lower, table.generation()));

        assert_eq!(
            table.probe_key(0, 0).entry().map(|entry| entry.depth()),
            Some(2),
            "the probed position is written back",
        );
        assert!(
            table.probe_key(intruder, 0).entry().is_none(),
            "the displaced position must not read the probed position's payload",
        );
        for slot in 1..super::BUCKET_SIZE as u64 {
            assert_eq!(
                table
                    .probe_key(colliding_key(&table, slot), 0)
                    .entry()
                    .map(|entry| entry.depth()),
                Some(8),
            );
        }
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
    /// that identifies its key. Half the stores decide from a probe taken before
    /// the hammering began, so their snapshots are as stale as a snapshot can
    /// be. Every successful probe must decode to the depth that key was stored
    /// with, which is what a torn read would violate.
    #[test]
    fn concurrent_writers_never_publish_a_torn_entry() {
        use std::sync::Arc;

        const WRITERS: u64 = 8;
        const ROUNDS: u64 = 4_000;

        let table = Arc::new(TranspositionTable::new(1).unwrap());
        table.start_search(0);
        let writers = (1..=WRITERS)
            .map(|writer| {
                let table = Arc::clone(&table);
                std::thread::spawn(move || {
                    let key = colliding_key(&table, writer);
                    let stale = table.probe_key(key, 0);
                    for round in 0..ROUNDS {
                        if round % 2 == 0 {
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
                        } else {
                            table.store_probed(
                                &stale,
                                writer as u32,
                                0,
                                writer as super::Score,
                                Bound::Exact,
                                None,
                                None,
                            );
                        }
                        for probe in 1..=WRITERS {
                            if let Some(entry) =
                                table.probe_key(colliding_key(&table, probe), 0).entry()
                            {
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
