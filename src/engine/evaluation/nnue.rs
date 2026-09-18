//! Experimental, standalone quantized neural evaluation.
//!
//! This module does not select an engine backend or supply a default network.
//! Features are colored piece-square pairs, including both kings, conditioned
//! on one of eight 2x2 buckets of the perspective king; one of eight output
//! layers is selected by the number of pieces on the board. Black's perspective
//! mirrors ranks; a perspective whose king stands on files e-h mirrors files as
//! well, so the king always lies on files a-d and every position shares
//! weights with its horizontal reflection. Both perspectives share the feature
//! transformer.
//! The output concatenates the side-to-move accumulator before its opponent's.
//!
//! Accumulators contain unclipped integer sums in 16 bits; the loader proves
//! that no placement can overflow them. Inference clips each activation
//! to 0..=255, computes an integer dot product, and divides by 255 * 64 with
//! truncation toward zero. The result is side-to-move-relative centipawns,
//! bounded below the engine's mate region. It is not a terminal-position score.

mod format;
mod kernels;
#[cfg(test)]
mod tests;

use std::fmt;

use cozy_chess::{BitBoard, Board, Color, Piece, Square};

pub use format::{FILE_BYTES, FORMAT_VERSION, HEADER_BYTES, LoadError, MAGIC};
use kernels::Row;

/// Number of 2x2 king buckets on the oriented, file-mirrored half board.
pub const KING_BUCKETS: usize = 8;
/// Own P/N/B/R/Q/K planes followed by the opponent's P/N/B/R/Q/K planes.
pub const PIECE_PLANES: usize = 12;
/// Number of feature-transformer rows.
pub const INPUT_FEATURES: usize = KING_BUCKETS * PIECE_PLANES * 64;
/// Width of each perspective's shared feature transformer.
pub const HIDDEN_SIZE: usize = 128;
/// Integer clipped-ReLU ceiling, and feature-transformer quantization scale.
pub const ACTIVATION_MAX: i32 = 255;
/// Quantization scale for centipawn-valued output weights.
pub const OUTPUT_SCALE: i32 = 64;
/// Output layers selected by piece count: `(pieces - 1) / 4` for 2..=32 pieces.
pub const OUTPUT_BUCKETS: usize = 8;
/// Largest absolute nonterminal score returned by this backend.
pub const MAX_SCORE: i32 = 16_000;

const _: () = assert!(MAX_SCORE < super::MATE_THRESHOLD);

const PIECES: [Piece; 6] = [
    Piece::Pawn,
    Piece::Knight,
    Piece::Bishop,
    Piece::Rook,
    Piece::Queen,
    Piece::King,
];
const COLORS: [Color; 2] = [Color::White, Color::Black];

/// Immutable, validated parameters for the fixed NNUE architecture.
///
/// Models can only be constructed by the bounded binary loaders. Mutable
/// accumulators borrow their model, so they cannot be reused with another set
/// of weights by accident. Loading never installs a model into an engine.
pub struct Network {
    hidden_bias: Row,
    /// One row per feature.
    input_weights: Box<[Row]>,
    /// Per output bucket: side-to-move row, then opponent row.
    output_weights: [[Row; 2]; OUTPUT_BUCKETS],
    output_bias: [i32; OUTPUT_BUCKETS],
}

impl fmt::Debug for Network {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Network")
            .field("input_features", &INPUT_FEATURES)
            .field("hidden_size", &HIDDEN_SIZE)
            .finish_non_exhaustive()
    }
}

impl Network {
    /// Fully recomputes the accumulators and returns a reference static score.
    #[must_use]
    pub fn evaluate(&self, board: &Board) -> i32 {
        self.accumulator(board).evaluate()
    }

    /// Builds independent mutable evaluation state bound to this model.
    #[must_use]
    pub fn accumulator(&self, board: &Board) -> Accumulator<'_> {
        let mut state = State::EMPTY;
        self.advance(&mut state, None, None, &FeaturePosition::new(board));
        Accumulator {
            network: self,
            state,
        }
    }

    /// Moves `target` to `next`.
    ///
    /// Every state records the placement its sums describe, so any state is a
    /// valid starting point. The sums start from whichever of `target` and
    /// `parent` shares more views with `next` and then differs from it in
    /// fewer features: a child is two or three features from its parent, a
    /// sibling four to six from the sibling evaluated before it, and either
    /// may be stale. A perspective whose view the chosen state does not share
    /// is refreshed.
    fn advance(
        &self,
        target: &mut State,
        parent: Option<&State>,
        mut cache: Option<&mut RefreshCache>,
        next: &FeaturePosition,
    ) {
        let own = Candidate::new(&target.position, next);
        let from = parent
            .map(|parent| (parent, Candidate::new(&parent.position, next)))
            .filter(|(_, candidate)| candidate.beats(&own));
        let (usable, diff) = from.map_or((own.usable, own.diff), |(_, candidate)| {
            (candidate.usable, candidate.diff)
        });
        if usable != [false; 2] && (from.is_some() || diff.changes() > 0) {
            let before = from.map_or(&target.position, |(parent, _)| &parent.position);
            let delta = Delta::new(&before.placement, &next.placement, diff, next.views);
            for index in (0..2).filter(|&index| usable[index]) {
                kernels::apply(
                    &self.input_weights,
                    &mut target.sums[index],
                    from.map(|(parent, _)| &parent.sums[index]),
                    delta.added(index),
                    delta.removed(index),
                );
            }
        }
        for index in (0..2).filter(|&index| !usable[index]) {
            self.refresh(&mut target.sums[index], index, cache.as_deref_mut(), next);
        }
        target.position = *next;
    }

    /// Recomputes one perspective whose view no nearby state shares.
    ///
    /// With many pieces on the board it starts from the sums last seen in this
    /// view, which are usually a few features away. With few it rebuilds: the
    /// views that change most are those of bare endgame kings, where summing
    /// the pieces left is cheaper than keeping the cache. Measured, an ungated
    /// cache cost 3% of endgame throughput for the 1% it earned elsewhere.
    #[inline(never)]
    fn refresh(
        &self,
        sums: &mut Row,
        index: usize,
        cache: Option<&mut RefreshCache>,
        next: &FeaturePosition,
    ) {
        let Some(cache) = cache.filter(|_| next.output_bucket >= RefreshCache::FIRST_BUCKET) else {
            return self.rebuild(sums, index, next);
        };
        let entry = &mut cache.entries[index][next.views[index].index()];
        let diff = entry.placement.diff(&next.placement);
        if diff.changes() <= MAX_DELTA {
            let delta = Delta::new(&entry.placement, &next.placement, diff, next.views);
            kernels::apply(
                &self.input_weights,
                &mut entry.sums,
                None,
                delta.added(index),
                delta.removed(index),
            );
        } else {
            self.rebuild(&mut entry.sums, index, next);
        }
        entry.placement = next.placement;
        *sums = entry.sums;
    }

    /// Sums every feature of one perspective onto the hidden bias: a delta
    /// from the empty placement, whose sums are the bias alone.
    fn rebuild(&self, sums: &mut Row, index: usize, next: &FeaturePosition) {
        let occupied = next.placement.colors[0] | next.placement.colors[1];
        let mut source = Some(&self.hidden_bias);
        // Either half of the board holds at most `MAX_DELTA` pieces.
        for half in [occupied & HALF_BOARD, occupied & !HALF_BOARD] {
            let diff = Diff {
                removed: 0,
                added: half,
            };
            let delta = Delta::new(&Placement::EMPTY, &next.placement, diff, next.views);
            kernels::apply(&self.input_weights, sums, source, delta.added(index), &[]);
            source = None;
        }
    }

    fn output(&self, state: &State) -> i32 {
        let position = &state.position;
        let ours = color_index(position.side_to_move);
        let numerator = self.output_bias[position.output_bucket]
            + kernels::output(
                &self.output_weights[position.output_bucket],
                [&state.sums[ours], &state.sums[1 - ours]],
            );
        (numerator / (ACTIVATION_MAX * OUTPUT_SCALE)).clamp(-MAX_SCORE, MAX_SCORE)
    }
}

/// Incrementally maintained state for exactly one immutable network.
///
/// Updating from the saved piece placement, rather than from an assumed parent
/// move, also supports sibling positions, skipped evaluations and search retries.
/// A perspective is fully refreshed when its king bucket or mirroring changes. Null moves
/// affect only output ordering; clocks, castling rights and en passant are not
/// features of this architecture.
#[derive(Clone)]
pub struct Accumulator<'network> {
    network: &'network Network,
    state: State,
}

impl Accumulator<'_> {
    /// Updates the exact unclipped sums to represent `board`.
    pub fn update(&mut self, board: &Board) {
        self.network
            .advance(&mut self.state, None, None, &FeaturePosition::new(board));
    }

    /// Returns the current side-to-move-relative static centipawn score.
    #[must_use]
    pub fn evaluate(&self) -> i32 {
        self.network.output(&self.state)
    }

    /// Exposes the exact, unclipped sums for exporter and reference validation.
    #[must_use]
    pub fn values(&self, perspective: Color) -> [i32; HIDDEN_SIZE] {
        self.state.values(perspective)
    }
}

/// One search worker's evaluation states, indexed by ply.
///
/// A state keeps the placement it was last asked about, so skipped parents,
/// same-ply re-searches and null moves need no push or pop hooks: evaluating a
/// board at a ply moves that ply's state to it from the cheaper of itself and
/// the state one ply up.
pub struct AccumulatorStack<'network> {
    network: &'network Network,
    states: Box<[State]>,
    cache: Box<RefreshCache>,
}

impl<'network> AccumulatorStack<'network> {
    /// Builds states for plies `0..=deepest`, all describing no placement yet.
    #[must_use]
    pub fn new(network: &'network Network, deepest: usize) -> Self {
        Self {
            network,
            states: vec![State::EMPTY; deepest + 1].into_boxed_slice(),
            cache: Box::new(RefreshCache::new(network)),
        }
    }

    /// Returns the side-to-move-relative static centipawn score of `board`.
    ///
    /// Plies beyond the deepest state share it.
    pub fn evaluate(&mut self, board: &Board, ply: usize) -> i32 {
        let ply = ply.min(self.states.len() - 1);
        let (above, below) = self.states.split_at_mut(ply);
        let target = &mut below[0];
        self.network.advance(
            target,
            above.last(),
            Some(&mut self.cache),
            &FeaturePosition::new(board),
        );
        self.network.output(target)
    }

    /// Exposes one ply's exact sums for reference validation.
    #[cfg(test)]
    pub(crate) fn values(&self, ply: usize, perspective: Color) -> [i32; HIDDEN_SIZE] {
        self.states[ply].values(perspective)
    }
}

/// Which piece stands where, as the board keeps it: the squares of each piece
/// type P/N/B/R/Q/K, then White's and Black's squares.
///
/// Absolute plane `color * 6 + kind` is the intersection of two of them.
#[derive(Clone, Copy)]
struct Placement {
    kinds: [u64; 6],
    colors: [u64; 2],
}

impl Placement {
    const EMPTY: Self = Self {
        kinds: [0; 6],
        colors: [0; 2],
    };

    fn new(board: &Board) -> Self {
        Self {
            kinds: PIECES.map(|piece| board.pieces(piece).0),
            colors: COLORS.map(|color| board.colors(color).0),
        }
    }

    fn plane(&self, plane: usize) -> u64 {
        self.colors[plane / 6] & self.kinds[plane % 6]
    }

    /// The owner and kind of the piece on an occupied square, without a
    /// branch per plane.
    fn piece_on(&self, square: usize) -> (usize, usize) {
        let on = |squares: u64| (squares >> square) as usize & 1;
        let kind = (1..6).map(|kind| kind * on(self.kinds[kind])).sum();
        (on(self.colors[1]), kind)
    }

    /// The squares whose piece leaves, and whose piece arrives, on the way to
    /// `after`. A square whose piece is replaced is in both.
    fn diff(&self, after: &Self) -> Diff {
        let mut changed = 0;
        for (before, after) in self.kinds.iter().zip(&after.kinds) {
            changed |= before ^ after;
        }
        for (before, after) in self.colors.iter().zip(&after.colors) {
            changed |= before ^ after;
        }
        Diff {
            removed: changed & (self.colors[0] | self.colors[1]),
            added: changed & (after.colors[0] | after.colors[1]),
        }
    }
}

#[derive(Clone, Copy)]
struct Diff {
    removed: u64,
    added: u64,
}

impl Diff {
    /// The number of features that differ, which is the same in every view.
    fn changes(self) -> u32 {
        self.removed.count_ones() + self.added.count_ones()
    }
}

/// The most feature changes applied incrementally; beyond it a perspective is
/// rebuilt. A legal position holds at most 32 pieces, so a longer list is
/// never the cheaper way to reach one.
const MAX_DELTA: u32 = 32;
/// Ranks one to four.
const HALF_BOARD: u64 = 0xffff_ffff;
const _: () = assert!(HALF_BOARD.count_ones() <= MAX_DELTA);

/// Both perspectives' sums and the placement they describe.
#[derive(Clone)]
struct State {
    position: FeaturePosition,
    sums: [Row; 2],
}

impl State {
    /// Describes no placement: neither view matches a real one, so the first
    /// use rebuilds both perspectives whatever the sums hold.
    const EMPTY: Self = Self {
        position: FeaturePosition {
            placement: Placement::EMPTY,
            views: [View::NONE; 2],
            side_to_move: Color::White,
            output_bucket: 0,
        },
        sums: [Row([0; HIDDEN_SIZE]); 2],
    };

    fn values(&self, perspective: Color) -> [i32; HIDDEN_SIZE] {
        self.sums[color_index(perspective)].map(i32::from)
    }
}

/// The sums last computed for each perspective in each view.
struct RefreshCache {
    entries: [[CacheEntry; View::COUNT]; 2],
}

#[derive(Clone, Copy)]
struct CacheEntry {
    placement: Placement,
    sums: Row,
}

impl RefreshCache {
    /// The first output bucket served: seventeen pieces and more.
    const FIRST_BUCKET: usize = 4;

    /// Every entry starts as the empty board, whose sums are the hidden bias.
    fn new(network: &Network) -> Self {
        let entry = CacheEntry {
            placement: Placement::EMPTY,
            sums: network.hidden_bias,
        };
        Self {
            entries: [[entry; View::COUNT]; 2],
        }
    }
}

/// A state the sums may start from: which perspectives share its view of the
/// next position, and the squares that differ.
#[derive(Clone, Copy)]
struct Candidate {
    usable: [bool; 2],
    diff: Diff,
}

impl Candidate {
    fn new(before: &FeaturePosition, next: &FeaturePosition) -> Self {
        let diff = before.placement.diff(&next.placement);
        let short = diff.changes() <= MAX_DELTA;
        Self {
            usable: [0, 1].map(|index| short && before.views[index] == next.views[index]),
            diff,
        }
    }

    /// Whether starting here is strictly cheaper than starting from `other`.
    fn beats(&self, other: &Self) -> bool {
        let shared = |candidate: &Self| candidate.usable.iter().filter(|&&usable| usable).count();
        (shared(other), self.diff.changes()) < (shared(self), other.diff.changes())
    }
}

/// Each perspective's features leaving and entering between two placements.
struct Delta {
    /// Per perspective: the features removed, then the features added.
    features: [[u16; MAX_DELTA as usize]; 2],
    removed: usize,
    total: usize,
}

impl Delta {
    /// Requires `diff.changes() <= MAX_DELTA`.
    fn new(before: &Placement, after: &Placement, diff: Diff, views: [View; 2]) -> Self {
        debug_assert!(diff.changes() <= MAX_DELTA);
        let mut features = [[0; MAX_DELTA as usize]; 2];
        let [white, black] = &mut features;
        let mut slots = white.iter_mut().zip(black);
        for (placement, squares) in [(before, diff.removed), (after, diff.added)] {
            for (square, (white, black)) in BitBoard(squares).into_iter().zip(&mut slots) {
                let (owner, kind) = placement.piece_on(square as usize);
                *white = views[0].feature(0, owner, kind, square as usize);
                *black = views[1].feature(1, owner, kind, square as usize);
            }
        }
        let removed = diff.removed.count_ones() as usize;
        Self {
            features,
            removed,
            total: removed + diff.added.count_ones() as usize,
        }
    }

    fn removed(&self, index: usize) -> &[u16] {
        &self.features[index][..self.removed]
    }

    fn added(&self, index: usize) -> &[u16] {
        &self.features[index][self.removed..self.total]
    }
}

/// How one perspective sees the board: its king bucket and whether files are
/// mirrored so that the king lies on files a-d.
#[derive(Clone, Copy, PartialEq, Eq)]
struct View {
    bucket: u8,
    mirror: bool,
}

impl View {
    /// Buckets, mirrored or not.
    const COUNT: usize = 2 * KING_BUCKETS;
    /// The view of no placement; it equals no real view.
    const NONE: Self = Self {
        bucket: u8::MAX,
        mirror: false,
    };

    fn index(self) -> usize {
        usize::from(self.bucket) + KING_BUCKETS * usize::from(self.mirror)
    }

    /// The feature of `owner`'s piece of `kind` on an absolute square, with
    /// the perspective and owner as color indices.
    fn feature(self, perspective: usize, owner: usize, kind: usize, square: usize) -> u16 {
        let flip = (56 * perspective) | (7 * usize::from(self.mirror));
        let plane = kind + 6 * (owner ^ perspective);
        ((usize::from(self.bucket) * PIECE_PLANES + plane) * 64 + (square ^ flip)) as u16
    }
}

#[derive(Clone, Copy)]
struct FeaturePosition {
    placement: Placement,
    views: [View; 2],
    side_to_move: Color,
    output_bucket: usize,
}

impl FeaturePosition {
    fn new(board: &Board) -> Self {
        Self {
            placement: Placement::new(board),
            views: std::array::from_fn(|index| view(board.king(COLORS[index]), COLORS[index])),
            side_to_move: board.side_to_move(),
            output_bucket: output_bucket(board),
        }
    }
}

/// Returns the sorted, unique feature indices for one perspective.
///
/// The feature number is `(bucket * 12 + relative_piece_plane) * 64 + square`,
/// with a1 = 0, P/N/B/R/Q/K = 0..5, and enemy planes offset by six. Black's
/// perspective XORs square indices with 56; a perspective whose king is on
/// files e-h XORs them with 7 as well, before selecting the bucket or row.
#[must_use]
pub fn active_features(board: &Board, perspective: Color) -> Vec<u16> {
    let position = FeaturePosition::new(board);
    let view = position.views[color_index(perspective)];
    let mut features = Vec::with_capacity(board.occupied().len() as usize);
    for plane in 0..PIECE_PLANES {
        for square in BitBoard(position.placement.plane(plane)) {
            features.push(view.feature(
                color_index(perspective),
                plane / 6,
                plane % 6,
                square as usize,
            ));
        }
    }
    features.sort_unstable();
    features
}

/// Selects the output layer from the piece count; both kings are always present.
#[must_use]
pub fn output_bucket(board: &Board) -> usize {
    (board.occupied().len() as usize - 1) / 4
}

fn color_index(color: Color) -> usize {
    match color {
        Color::White => 0,
        Color::Black => 1,
    }
}

fn oriented_square(square: usize, perspective: Color, mirror: bool) -> usize {
    let flip = if perspective == Color::Black { 56 } else { 0 } | if mirror { 7 } else { 0 };
    square ^ flip
}

fn view(king: Square, perspective: Color) -> View {
    let mirror = king.file() as usize >= 4;
    let square = oriented_square(king as usize, perspective, mirror);
    View {
        bucket: ((square / 8 / 2) * 2 + (square % 8 / 2)) as u8,
        mirror,
    }
}
