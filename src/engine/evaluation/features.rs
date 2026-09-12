use std::cell::Cell;

use cozy_chess::{
    BitBoard, Board, Color, File, Piece, Rank, Square, get_between_rays, get_bishop_moves,
    get_bishop_rays, get_king_moves, get_knight_moves, get_line_rays, get_pawn_attacks,
    get_rook_moves, get_rook_rays,
};

use super::{
    AttackProfile, EvalFeatures, KING_DANGER_BUCKETS, ScorePair, piece_value, placement, weights,
};

/// Attack units a piece contributes when it attacks the enemy king zone, on
/// top of one unit per zone square it attacks.
#[inline(always)]
const fn king_attack_units(piece: Piece) -> i32 {
    match piece {
        Piece::Pawn => 1,
        Piece::Knight | Piece::Bishop => 2,
        Piece::Rook => 3,
        Piece::Queen => 5,
        Piece::King => 0,
    }
}

/// Maps attack units onto the danger table's buckets.
///
/// Two units per bucket keeps a lone minor piece with one zone square in the
/// first bucket and a full assault of queen, rook and both minors in the top
/// few, which is the range a fit has to describe.
pub(super) const fn king_danger_bucket(units: i32) -> usize {
    let bucket = (units / 2) as usize;
    if bucket >= KING_DANGER_BUCKETS {
        KING_DANGER_BUCKETS - 1
    } else {
        bucket
    }
}

/// Files ahead of each square from White's perspective, on the file and both
/// neighbours.
///
/// A pawn is passed when no enemy pawn stands anywhere in this span, which is a
/// single mask test in place of a scan over every enemy pawn.
static WHITE_PASSER_SPANS: [BitBoard; 64] = build_passer_spans(true);
static BLACK_PASSER_SPANS: [BitBoard; 64] = build_passer_spans(false);
/// Files adjacent to each square, ahead of it from White's perspective.
///
/// An enemy pawn anywhere in this span could one day attack the square, so a
/// piece there is not on an outpost. It is the passer span without the
/// square's own file.
static WHITE_OUTPOST_CHALLENGES: [BitBoard; 64] = build_outpost_challenges(true);
static BLACK_OUTPOST_CHALLENGES: [BitBoard; 64] = build_outpost_challenges(false);
/// Squares one and two ranks ahead of each square, on the file and both
/// neighbours, used for king shelter.
static WHITE_SHELTER_ZONES: [BitBoard; 64] = build_shelter_zones(true);
static BLACK_SHELTER_ZONES: [BitBoard; 64] = build_shelter_zones(false);
/// The file of each square together with both neighbouring files.
static KING_FILE_SPANS: [BitBoard; 64] = build_king_file_spans();
/// The four centre files on a side's own second to fourth ranks, where its
/// space is counted.
static WHITE_SPACE_ZONE: BitBoard = build_space_zone(true);
static BLACK_SPACE_ZONE: BitBoard = build_space_zone(false);
/// The four files nearest a king on each file: a to d for a king on a to c,
/// c to f for one on d or e, e to h for one on f to h.
static KING_FLANKS: [BitBoard; 8] = build_king_flanks();
/// The light squares, b1 among them.
const LIGHT_SQUARES: BitBoard = BitBoard(0x55aa_55aa_55aa_55aa);

const fn square_mask(file: usize, rank: usize) -> u64 {
    1_u64 << (rank * 8 + file)
}

const fn build_passer_spans(white: bool) -> [BitBoard; 64] {
    let mut spans = [BitBoard::EMPTY; 64];
    let mut index = 0;
    while index < 64 {
        let file = index % 8;
        let rank = index / 8;
        let mut mask = 0_u64;
        let mut other_file = if file == 0 { 0 } else { file - 1 };
        let last_file = if file == 7 { 7 } else { file + 1 };
        while other_file <= last_file {
            let mut other_rank = 0;
            while other_rank < 8 {
                let ahead = if white {
                    other_rank > rank
                } else {
                    other_rank < rank
                };
                if ahead {
                    mask |= square_mask(other_file, other_rank);
                }
                other_rank += 1;
            }
            other_file += 1;
        }
        spans[index] = BitBoard(mask);
        index += 1;
    }
    spans
}

const fn build_outpost_challenges(white: bool) -> [BitBoard; 64] {
    let spans = build_passer_spans(white);
    let mut challenges = [BitBoard::EMPTY; 64];
    let mut index = 0;
    while index < 64 {
        let file = index % 8;
        let mut own_file = 0_u64;
        let mut rank = 0;
        while rank < 8 {
            own_file |= square_mask(file, rank);
            rank += 1;
        }
        challenges[index] = BitBoard(spans[index].0 & !own_file);
        index += 1;
    }
    challenges
}

const fn build_shelter_zones(white: bool) -> [BitBoard; 64] {
    let mut zones = [BitBoard::EMPTY; 64];
    let mut index = 0_usize;
    while index < 64 {
        let file = index % 8;
        let rank = index / 8;
        let mut mask = 0_u64;
        let mut other_file = if file == 0 { 0 } else { file - 1 };
        let last_file = if file == 7 { 7 } else { file + 1 };
        while other_file <= last_file {
            let mut step = 1_usize;
            while step <= 2 {
                let target = if white {
                    rank + step
                } else {
                    rank.wrapping_sub(step)
                };
                if target < 8 {
                    mask |= square_mask(other_file, target);
                }
                step += 1;
            }
            other_file += 1;
        }
        zones[index] = BitBoard(mask);
        index += 1;
    }
    zones
}

const fn build_king_flanks() -> [BitBoard; 8] {
    let mut flanks = [BitBoard::EMPTY; 8];
    let mut king_file = 0;
    while king_file < 8 {
        let first = if king_file <= 2 {
            0
        } else if king_file <= 4 {
            2
        } else {
            4
        };
        let mut mask = 0_u64;
        let mut file = first;
        while file < first + 4 {
            let mut rank = 0;
            while rank < 8 {
                mask |= square_mask(file, rank);
                rank += 1;
            }
            file += 1;
        }
        flanks[king_file] = BitBoard(mask);
        king_file += 1;
    }
    flanks
}

const fn build_space_zone(white: bool) -> BitBoard {
    let mut mask = 0_u64;
    let mut file = 2;
    while file <= 5 {
        let mut step = 1;
        while step <= 3 {
            let rank = if white { step } else { 7 - step };
            mask |= square_mask(file, rank);
            step += 1;
        }
        file += 1;
    }
    BitBoard(mask)
}

const fn build_king_file_spans() -> [BitBoard; 64] {
    let mut spans = [BitBoard::EMPTY; 64];
    let mut index = 0;
    while index < 64 {
        let file = index % 8;
        let mut mask = 0_u64;
        let mut other_file = if file == 0 { 0 } else { file - 1 };
        let last_file = if file == 7 { 7 } else { file + 1 };
        while other_file <= last_file {
            let mut rank = 0;
            while rank < 8 {
                mask |= square_mask(other_file, rank);
                rank += 1;
            }
            other_file += 1;
        }
        spans[index] = BitBoard(mask);
        index += 1;
    }
    spans
}

/// Pawn and king structure for one position, as side-relative feature deltas.
///
/// Every field is a function of the pawn placement of both colours and the two
/// king squares alone, which is what makes it cacheable across nodes: the rest of
/// the evaluation changes when any piece moves, but these terms change only when
/// a pawn or a king does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct StructureTerms {
    doubled: i32,
    isolated: i32,
    backward: i32,
    /// Passers weighted by how far they have come, which the attacking style
    /// reads. Derived from the per-rank counts so it cannot drift from them.
    passed_pawns: i32,
    shelter: i32,
    open_files: i32,
    /// Every rank- or distance-indexed structure block, already weighted.
    ///
    /// The counts behind it — passers, protected passers and connected pawns
    /// by rank, passers by each king's distance, shelter and storm by pawn
    /// distance — are eighty entries that the engine would otherwise copy out of the
    /// cache and multiply by their weights at every node, almost all of them
    /// zero. Weighting them once, on the miss, makes a hit one pair. The
    /// counts themselves are recomputed by [`structure_counts`] for the
    /// fitter and the tests, which are the only readers that need them.
    indexed: ScorePair,
    /// Each colour's safe centre squares behind its pawns, raw rather than
    /// weighted, because the extraction multiplies them by a piece count
    /// that is not a function of the pawns.
    space_area: [i8; 2],
}

/// The pawn and king structure counts, side-relative, as the fitter and the
/// tests read them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct StructureCounts {
    pub(super) doubled: i32,
    pub(super) isolated: i32,
    pub(super) backward: i32,
    /// Passed pawns counted per rank, from the owner's side of the board. A
    /// pawn can only stand on relative ranks one to six.
    pub(super) passed_by_rank: [i32; 6],
    /// Passed pawns defended by a friendly pawn, counted the same way.
    pub(super) protected_passer_by_rank: [i32; 6],
    /// Connected pawns, counted the same way.
    pub(super) connected_by_rank: [i32; 6],
    /// Passers by the distance from their owner's king to the square in
    /// front of them, and by the distance from the enemy king to it.
    pub(super) passer_own_king_distance: [i32; 8],
    pub(super) passer_enemy_king_distance: [i32; 8],
    pub(super) shelter: i32,
    pub(super) open_files: i32,
    /// The nearest friendly pawn ahead of the king on its own file and on
    /// each adjacent file, by rank distance.
    pub(super) shelter_king_file_by_distance: [i32; 6],
    pub(super) shelter_adjacent_file_by_distance: [i32; 6],
    /// The nearest enemy pawn ahead of the king on its own file and on each
    /// adjacent file, by rank distance, with the pawns a friendly pawn
    /// stands directly in front of counted apart from both.
    pub(super) storm_king_file_by_distance: [i32; 6],
    pub(super) storm_adjacent_file_by_distance: [i32; 6],
    pub(super) blocked_storm_by_distance: [i32; 6],
    /// Each colour's safe centre squares behind its pawns, by colour rather
    /// than side-relative, since the weight they earn depends on the
    /// owner's pieces.
    pub(super) space_area: [i32; 2],
    /// Pawns not passed whose own file ahead is clear and whose helpers on
    /// the adjacent files can match the enemy pawns ahead there, by rank.
    pub(super) candidate_passer_by_rank: [i32; 6],
    /// Runs of adjacent files holding a pawn.
    pub(super) pawn_islands: i32,
    /// Kings whose four-file flank holds no pawn of either colour.
    pub(super) king_pawnless_flank: i32,
}

/// The inputs a [`StructureTerms`] depends on, stored so a hit is exact.
///
/// Verifying the full inputs rather than a hash means a cache hit returns the
/// value recomputation would have produced, so the cache cannot change a score.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct StructureKey {
    white_pawns: u64,
    black_pawns: u64,
    white_king: u8,
    black_king: u8,
}

impl StructureKey {
    fn new(board: &Board) -> Self {
        Self {
            white_pawns: board.colored_pieces(Color::White, Piece::Pawn).0,
            black_pawns: board.colored_pieces(Color::Black, Piece::Pawn).0,
            white_king: board.king(Color::White) as u8,
            black_king: board.king(Color::Black) as u8,
        }
    }

    /// Returns the table slot this key maps to.
    ///
    /// The pawn bitboards carry nearly all of the entropy, so they are mixed with
    /// a multiplicative hash and the king squares folded in afterwards.
    fn slot(self) -> usize {
        let mut hash = self.white_pawns.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        hash ^= self.black_pawns.wrapping_mul(0xbf58_476d_1ce4_e5b9);
        hash ^= u64::from(self.white_king) << 7 | u64::from(self.black_king) << 1;
        hash = hash.wrapping_mul(0x94d0_49bb_1331_11eb);
        (hash >> 40) as usize & (STRUCTURE_CACHE_SLOTS - 1)
    }
}

/// Slots in the direct-mapped structure cache.
///
/// Sized to stay inside a core's private cache: at 32 bytes an entry, this is
/// 64 KiB, which a search revisits far more often than it evicts.
const STRUCTURE_CACHE_SLOTS: usize = 2048;

#[derive(Clone, Copy, Default)]
struct StructureCacheEntry {
    key: StructureKey,
    terms: StructureTerms,
}

thread_local! {
    /// Per-thread structure cache.
    ///
    /// Evaluation is a pure function of the board, so the cache is an
    /// optimization with no observable effect and needs no sharing between
    /// threads. Keeping it thread-local also keeps the search deterministic:
    /// whatever the cache state, a hit is verified against the full key.
    static STRUCTURE_CACHE: Box<[Cell<StructureCacheEntry>]> = {
        let mut entries = Vec::with_capacity(STRUCTURE_CACHE_SLOTS);
        for _ in 0..STRUCTURE_CACHE_SLOTS {
            entries.push(Cell::new(StructureCacheEntry::default()));
        }
        entries.into_boxed_slice()
    };
}

/// Returns pawn and king structure terms, computing them only on a miss.
///
/// The empty key cannot collide with a real position, because every legal
/// position has two kings and `Square::A1` is square zero only for one of them at
/// a time; a slot still holding the default is simply a miss and is recomputed.
#[inline(always)]
fn structure_terms(board: &Board) -> StructureTerms {
    let key = StructureKey::new(board);
    let slot = key.slot();
    STRUCTURE_CACHE.with(|cache| {
        let entry = cache[slot].get();
        if entry.key == key {
            return entry.terms;
        }
        let terms = compute_structure_terms(board);
        cache[slot].set(StructureCacheEntry { key, terms });
        terms
    })
}

/// Computes pawn and king structure terms from scratch.
fn compute_structure_terms(board: &Board) -> StructureTerms {
    let counts = structure_counts(board);
    StructureTerms {
        doubled: counts.doubled,
        isolated: counts.isolated,
        backward: counts.backward,
        passed_pawns: (0..6)
            .map(|rank| (rank as i32 + 1) * counts.passed_by_rank[rank])
            .sum(),
        shelter: counts.shelter,
        open_files: counts.open_files,
        indexed: weights::structure_indexed(&counts),
        space_area: [counts.space_area[0] as i8, counts.space_area[1] as i8],
    }
}

/// Computes every pawn and king structure count from scratch.
pub(super) fn structure_counts(board: &Board) -> StructureCounts {
    let mut counts = StructureCounts::default();
    for color in [Color::White, Color::Black] {
        let sign = if color == Color::White { 1 } else { -1 };
        let pawns = pawn_features(board, color);
        counts.doubled += sign * pawns.doubled;
        counts.isolated += sign * pawns.isolated;
        counts.backward += sign * pawns.backward;
        for rank in 0..6 {
            counts.passed_by_rank[rank] += sign * i32::from(pawns.passed_by_rank[rank]);
            counts.protected_passer_by_rank[rank] +=
                sign * i32::from(pawns.protected_passer_by_rank[rank]);
            counts.connected_by_rank[rank] += sign * i32::from(pawns.connected_by_rank[rank]);
            counts.candidate_passer_by_rank[rank] +=
                sign * i32::from(pawns.candidate_by_rank[rank]);
        }
        counts.pawn_islands += sign * pawns.islands;
        counts.king_pawnless_flank += sign * pawnless_flank(board, color);
        let (shelter, open_files) = king_safety(board, color);
        counts.shelter += sign * shelter;
        counts.open_files += sign * open_files;
        let (king_file, adjacent) = shelter_distances(board, color);
        for distance in 0..6 {
            counts.shelter_king_file_by_distance[distance] += sign * i32::from(king_file[distance]);
            counts.shelter_adjacent_file_by_distance[distance] +=
                sign * i32::from(adjacent[distance]);
        }
        let (king_file, adjacent, blocked) = storm_distances(board, color);
        for distance in 0..6 {
            counts.storm_king_file_by_distance[distance] += sign * i32::from(king_file[distance]);
            counts.storm_adjacent_file_by_distance[distance] +=
                sign * i32::from(adjacent[distance]);
            counts.blocked_storm_by_distance[distance] += sign * i32::from(blocked[distance]);
        }
        counts.space_area[color as usize] = i32::from(space_area(board, color));
        let (own, enemy) = passer_king_distances(board, color);
        for distance in 0..8 {
            counts.passer_own_king_distance[distance] += sign * i32::from(own[distance]);
            counts.passer_enemy_king_distance[distance] += sign * i32::from(enemy[distance]);
        }
    }
    counts
}

/// Counts a colour's passers by each king's distance to the square ahead.
///
/// The square in front is what a king must reach to stop or escort a passer,
/// so it is the square the distance is measured to. Chebyshev distance is the
/// king's own metric. The counts are functions of the pawns and the two king
/// squares, so they ride the structure cache.
fn passer_king_distances(board: &Board, color: Color) -> ([i8; 8], [i8; 8]) {
    let pawns = board.colored_pieces(color, Piece::Pawn);
    let enemy_pawns = board.colored_pieces(!color, Piece::Pawn);
    let (spans, forward) = if color == Color::White {
        (&WHITE_PASSER_SPANS, 1)
    } else {
        (&BLACK_PASSER_SPANS, -1)
    };
    let own_king = board.king(color);
    let enemy_king = board.king(!color);
    let mut own = [0_i8; 8];
    let mut enemy = [0_i8; 8];
    for square in pawns {
        if !(enemy_pawns & spans[square as usize]).is_empty() {
            continue;
        }
        // A passer stands on ranks two to seven, so the square ahead exists.
        let Some(stop) = square.try_offset(0, forward) else {
            continue;
        };
        own[king_distance(own_king, stop)] += 1;
        enemy[king_distance(enemy_king, stop)] += 1;
    }
    (own, enemy)
}

/// Number of king moves between two squares.
fn king_distance(from: Square, to: Square) -> usize {
    let files = (from.file() as i32 - to.file() as i32).unsigned_abs();
    let ranks = (from.rank() as i32 - to.rank() as i32).unsigned_abs();
    files.max(ranks) as usize
}

/// A conservative pawn-only race advantage, not a forced-conversion score.
/// Tempo and en passant make this unsuitable for the structure cache.
fn pawn_race(board: &Board) -> i32 {
    let pawns = board.pieces(Piece::Pawn);
    if pawns.is_empty()
        || board.occupied() != (pawns | board.pieces(Piece::King))
        || !board.checkers().is_empty()
        || board.en_passant().is_some()
    {
        return 0;
    }
    let mut advantage = 0;
    for color in [Color::White, Color::Black] {
        let enemy_pawns = board.colored_pieces(!color, Piece::Pawn);
        let earliest_enemy = enemy_pawns
            .into_iter()
            .map(|pawn| optimistic_promotion_ply(pawn, !color, board.side_to_move()))
            .min();
        let ahead = board
            .colored_pieces(color, Piece::Pawn)
            .into_iter()
            .any(|pawn| {
                let promotion_ply = optimistic_promotion_ply(pawn, color, board.side_to_move());
                // Even blocked or catchable enemy pawns veto a close race. Checks
                // on promotion and adjacent-ply races belong to the search.
                if earliest_enemy.is_some_and(|ply| ply <= promotion_ply + 2) {
                    return false;
                }
                clear_pawn_run(board, pawn, color, promotion_ply)
            });
        advantage += if color == Color::White {
            i32::from(ahead)
        } else {
            -i32::from(ahead)
        };
    }
    advantage
}

fn optimistic_promotion_ply(pawn: Square, color: Color, side_to_move: Color) -> u8 {
    let rank = if color == Color::White {
        pawn.rank() as u8
    } else {
        7 - pawn.rank() as u8
    };
    let pushes = 7 - rank - u8::from(rank == 1);
    2 * pushes - u8::from(color == side_to_move)
}

/// Overestimate enemy pawn reach by ignoring blockers and allowing hypothetical
/// captures. Suppressing a bonus is preferable to inventing a safe running plan.
fn optimistic_pawn_reach(pawns: BitBoard, color: Color) -> BitBoard {
    let (single, double) = if color == Color::White {
        (
            BitBoard(pawns.0 << 8),
            BitBoard((pawns & Rank::Second.bitboard()).0 << 16),
        )
    } else {
        (
            BitBoard(pawns.0 >> 8),
            BitBoard((pawns & Rank::Seventh.bitboard()).0 >> 16),
        )
    };
    (pawns | single | double | pawn_attack_set(pawns, color))
        & !(Rank::First.bitboard() | Rank::Eighth.bitboard())
}

fn clear_pawn_run(board: &Board, pawn: Square, color: Color, promotion_ply: u8) -> bool {
    let enemy = !color;
    let enemy_pawns = board.colored_pieces(enemy, Piece::Pawn);
    let (spans, forward, start, promotion) = if color == Color::White {
        (&WHITE_PASSER_SPANS, 1, Rank::Second, Rank::Eighth)
    } else {
        (&BLACK_PASSER_SPANS, -1, Rank::Seventh, Rank::First)
    };
    if !(enemy_pawns & spans[pawn as usize]).is_empty() {
        return false;
    }
    let path = spans[pawn as usize] & pawn.file().bitboard();
    if !(path & board.occupied()).is_empty() {
        return false;
    }
    let enemy_king = board.king(enemy);
    let own_king = board.king(color);
    let defender_first = u8::from(board.side_to_move() == enemy);
    if defender_first != 0 && king_distance(enemy_king, pawn) <= 1 {
        return false;
    }
    let mut enemy_reach = enemy_pawns;
    if defender_first != 0 {
        enemy_reach = optimistic_pawn_reach(enemy_reach, enemy);
        if pawn_attack_set(enemy_reach, enemy).has(own_king) {
            return false;
        }
    }
    let mut square = pawn;
    let mut pushes = 0_u8;
    while square.rank() != promotion {
        let Some(next) = square.try_offset(0, forward) else {
            return false;
        };
        if enemy_reach.has(next) {
            return false;
        }
        let landing = if square == pawn && square.rank() == start {
            if defender_first != 0 && king_distance(enemy_king, next) <= 1 {
                return false;
            }
            let Some(double) = next.try_offset(0, forward) else {
                return false;
            };
            double
        } else {
            next
        };
        pushes += 1;
        // Give the defending king an unobstructed route and ignore friendly
        // protection. Include its reply after promotion, not just the push.
        if king_distance(enemy_king, landing) <= usize::from(pushes + defender_first)
            || enemy_reach.has(landing)
        {
            return false;
        }
        enemy_reach = optimistic_pawn_reach(enemy_reach, enemy);
        let attacks = pawn_attack_set(enemy_reach, enemy);
        if attacks.has(landing) {
            return false;
        }
        if 2 * pushes - 1 + defender_first < promotion_ply && attacks.has(own_king) {
            return false;
        }
        square = landing;
    }
    true
}

pub(super) fn extract(board: &Board) -> EvalFeatures {
    extract_with_style(board, true)
}

/// Extracts evaluation features, computing style-only attack terms on request.
///
/// Mobility and material features are always produced. When `style` is false the
/// king-pressure, threat, space, and supported-threat terms are left at their
/// defaults, which is sound only for configurations that weight them at zero.
pub(super) fn extract_with_style(board: &Board, style: bool) -> EvalFeatures {
    let mut features = EvalFeatures::default();
    let attacks = attack_summary_with_style(board, style);
    let white_attack = attacks.scans[Color::White as usize].profile;
    let black_attack = attacks.scans[Color::Black as usize].profile;
    features.white_attack = white_attack;
    features.black_attack = black_attack;

    // Pawn and king structure depends only on pawn placement and king squares, so
    // it is accumulated for both colours at once through the cache.
    let structure = structure_terms(board);

    for color in [Color::White, Color::Black] {
        let sign = if color == Color::White { 1 } else { -1 };
        let knights = board.colored_pieces(color, Piece::Knight).len() as i32;
        let bishops = board.colored_pieces(color, Piece::Bishop).len() as i32;
        let rooks = board.colored_pieces(color, Piece::Rook).len() as i32;
        let queens = board.colored_pieces(color, Piece::Queen).len() as i32;
        let pawns = board.colored_pieces(color, Piece::Pawn).len() as i32;
        features.pawns += sign * pawns;
        features.knights += sign * knights;
        features.bishops += sign * bishops;
        features.rooks += sign * rooks;
        features.queens += sign * queens;
        features.bishop_pair += sign * i32::from(bishops >= 2);
        // Room is worth what wants to use it: the cached count is carried
        // once as it is and once scaled by the owner's pieces.
        let area = i32::from(structure.space_area[color as usize]);
        features.space_area += sign * area;
        features.space_area_by_pieces += sign * area * (knights + bishops + rooks + queens);
        let scan = &attacks.scans[color as usize];
        // A piece's worth bends with the pawn count, and a bishop's with the
        // pawns on its colour: products the fit weights, linear in the
        // weights.
        features.knight_pawns += sign * knights * pawns;
        features.bishop_pawns += sign * bishops * pawns;
        features.rook_pawns += sign * rooks * pawns;
        features.bishop_pawns_on_colour += sign * scan.bishop_pawns_on_colour;
        features.activity += sign * scan.activity;
        features.placement = features.placement + scan.placement * sign;
        features.mobility += sign * scan.mobility;
        features.pawn_mobility += sign * scan.piece_mobility[piece_index(Piece::Pawn) as usize];
        features.knight_mobility += sign * scan.piece_mobility[piece_index(Piece::Knight) as usize];
        features.bishop_mobility += sign * scan.piece_mobility[piece_index(Piece::Bishop) as usize];
        features.rook_mobility += sign * scan.piece_mobility[piece_index(Piece::Rook) as usize];
        features.queen_mobility += sign * scan.piece_mobility[piece_index(Piece::Queen) as usize];
        features.king_mobility += sign * scan.piece_mobility[piece_index(Piece::King) as usize];
        features.mobility_curves = features.mobility_curves + scan.mobility_curves * sign;
        for (total, &count) in features
            .unsafe_mobility
            .iter_mut()
            .zip(&scan.unsafe_mobility)
        {
            *total += sign * count;
        }
        features.piece_indexed = features.piece_indexed + scan.tropism * sign;
        let [open, semi_open, seventh] = scan.rook_files;
        features.rook_open_files += sign * open;
        features.rook_semi_open_files += sign * semi_open;
        features.rooks_on_seventh += sign * seventh;
        let [knights, bishops] = scan.outposts;
        features.knight_outposts += sign * knights;
        features.bishop_outposts += sign * bishops;
        // The piece-loop blocks indexed by rank, bucket or piece are weighted
        // here, as placement and the mobility curves are in the loop, rather
        // than carried as counts for the scorer to multiply.
        for (rank, &blocked) in attacks.blocked_passers[color as usize].iter().enumerate() {
            if blocked != 0 {
                features.piece_indexed = features.piece_indexed
                    + weights::blocked_passer_weight(rank) * (sign * blocked);
            }
        }
        features.piece_indexed =
            features.piece_indexed + attacks.passer_path[color as usize] * sign;
        features.rook_behind_passer += sign * attacks.rook_behind_passer[color as usize];
        features.threat_by_pawn_push += sign * attacks.pawn_push_threats[color as usize];
        let rights = board.castle_rights(color);
        features.castling_rights +=
            sign * (i32::from(rights.short.is_some()) + i32::from(rights.long.is_some()));
        // A colour that brings nothing against the enemy king is not counted
        // in the first bucket: the term describes an attack, not its absence.
        let units = scan.attack_units();
        if units > 0 {
            features.piece_indexed = features.piece_indexed
                + weights::king_danger_weight(king_danger_bucket(units)) * sign;
        }
        for (slot, &checks) in attacks.safe_checks[color as usize].iter().enumerate() {
            if checks != 0 {
                features.piece_indexed =
                    features.piece_indexed + weights::safe_check_weight(slot) * (sign * checks);
            }
        }
        let [by_pawn, hanging, by_lower] = attacks.threats[color as usize];
        features.threat_minor_by_pawn += sign * by_pawn;
        features.threat_hanging += sign * hanging;
        features.threat_by_lower_value += sign * by_lower;
        if style {
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
        }
    }

    features.doubled_pawns = structure.doubled;
    features.isolated_pawns = structure.isolated;
    features.backward_pawns = structure.backward;
    // The attacking style weights passers by how far they have come, and that
    // term is personality and must not move. It is derived from the per-rank
    // counts on the cache miss, so it cannot drift from them.
    features.passed_pawns = structure.passed_pawns;
    features.king_shelter = structure.shelter;
    features.open_king_files = structure.open_files;
    features.structure_indexed = structure.indexed;
    features.pawn_race = pawn_race(board);

    features.tempo = if board.side_to_move() == Color::White {
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

/// Reference activity accumulation, retained to check the fused piece pass.
#[cfg(test)]
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

static CENTRALITY: [i32; 64] = build_centrality_table();

const fn build_centrality_table() -> [i32; 64] {
    let mut table = [0; 64];
    let mut index = 0;
    while index < 64 {
        let file = (index % 8) as i32;
        let rank = (index / 8) as i32;
        let file_dist_3 = (file - 3).abs();
        let file_dist_4 = (file - 4).abs();
        let file_distance = if file_dist_3 < file_dist_4 {
            file_dist_3
        } else {
            file_dist_4
        };
        let rank_dist_3 = (rank - 3).abs();
        let rank_dist_4 = (rank - 4).abs();
        let rank_distance = if rank_dist_3 < rank_dist_4 {
            rank_dist_3
        } else {
            rank_dist_4
        };
        table[index] = 6 - file_distance - rank_distance;
        index += 1;
    }
    table
}

#[inline(always)]
fn centrality(square: Square) -> i32 {
    CENTRALITY[square as usize]
}

#[cfg(test)]
fn reference_mobility(board: &Board, color: Color) -> i32 {
    let occupied = board.occupied();
    let friendly = board.colors(color);
    let pinned = pinned_pieces(board, color);
    let king = board.king(color);
    let mut total = 0;
    for piece in Piece::ALL {
        for square in board.colored_pieces(color, piece) {
            let attacks = attacks_from(piece, square, color, occupied);
            total +=
                (pin_restricted_attacks(attacks, square, king, pinned) & !friendly).len() as i32;
        }
    }
    total
}

/// Finds absolute pins for either side, independent of the side to move.
fn pinned_pieces(board: &Board, color: Color) -> BitBoard {
    let king = board.king(color);
    let enemy = !color;
    let queens = board.colored_pieces(enemy, Piece::Queen);
    let diagonal = board.colored_pieces(enemy, Piece::Bishop) | queens;
    let orthogonal = board.colored_pieces(enemy, Piece::Rook) | queens;
    let snipers = (get_bishop_rays(king) & diagonal) | (get_rook_rays(king) & orthogonal);
    let mut pinned = BitBoard::EMPTY;
    for sniper in snipers {
        let blockers = get_between_rays(king, sniper) & board.occupied();
        if blockers.len() == 1 {
            pinned |= blockers & board.colors(color);
        }
    }
    pinned
}

#[inline(always)]
fn pin_restricted_attacks(
    attacks: BitBoard,
    square: Square,
    king: Square,
    pinned: BitBoard,
) -> BitBoard {
    if pinned.has(square) {
        attacks & get_line_rays(king, square)
    } else {
        attacks
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct AttackSummary {
    /// Each colour's scan, White first.
    pub(super) scans: [ColourScan; 2],
    /// Passers with a piece of either colour on the square ahead, by rank.
    pub(super) blocked_passers: [[i32; 6]; 2],
    /// Safe checking squares available to each colour's knights, bishops,
    /// rooks and queens.
    pub(super) safe_checks: [[i32; 4]; 2],
    /// Enemy minors attacked by a pawn, enemy pieces attacked and undefended,
    /// and enemy pieces attacked by something worth less, per colour.
    pub(super) threats: [[i32; 3]; 2],
    /// Passers whose path to promotion no enemy piece attacks, and passers
    /// whose path no piece stands on, by rank; and passers with a friendly
    /// rook behind them on the file with nothing between.
    pub(super) passer_safe_path: [[i32; 6]; 2],
    pub(super) passer_free_path: [[i32; 6]; 2],
    pub(super) rook_behind_passer: [i32; 2],
    /// The two path blocks above already weighted, per colour, so the
    /// extraction adds one pair rather than walking twelve counts that are
    /// almost always zero; the counts are for the fitter and the tests.
    pub(super) passer_path: [ScorePair; 2],
    /// Enemy pieces, pawns and king aside, a friendly pawn would attack
    /// after a safe push, per colour.
    pub(super) pawn_push_threats: [i32; 2],
}

#[cfg(test)]
pub(super) fn attack_summary(board: &Board) -> AttackSummary {
    attack_summary_with_style(board, true)
}

/// What one colour's pieces accumulate in the fused pass.
///
/// One struct per colour rather than one `[T; 2]` per term: the square loop
/// then writes to fields of a single local rather than indexing a dozen
/// arrays, which is what keeps its live values in registers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ColourScan {
    pub(super) profile: AttackProfile,
    attacker_mask: u8,
    zone_defenders: i32,
    pub(super) mobility: i32,
    pub(super) piece_mobility: [i32; 6],
    pub(super) mobility_curves: ScorePair,
    pub(super) rook_files: [i32; 3],
    pub(super) outposts: [i32; 2],
    passers: BitBoard,
    /// Outpost-rank squares a friendly pawn defends, known once the pawns
    /// have been scanned and before the minors are.
    pawn_held: BitBoard,
    /// Zone squares attacked, summed over pieces, and the type units of the
    /// pieces attacking any; their sum is the attack-unit total.
    zone_hits: i32,
    attacker_units: i32,
    pub(super) attacked: BitBoard,
    pub(super) attacked_twice: BitBoard,
    pub(super) type_attacks: [BitBoard; 4],
    pub(super) pawn_attacks: BitBoard,
    pub(super) activity: i32,
    pub(super) placement: ScorePair,
    /// Friendly pawns on the square colour of each bishop, summed.
    pub(super) bishop_pawns_on_colour: i32,
    /// Moves onto squares an enemy pawn attacks, for knights, bishops,
    /// rooks and queens in turn.
    pub(super) unsafe_mobility: [i32; 4],
    /// The tropism block already weighted: each knight, bishop, rook and
    /// queen by its bucketed distance to the enemy king, looked up in the
    /// loop as the mobility curves are.
    pub(super) tropism: ScorePair,
    /// Pin-restricted reach for direct threats, separate from king safety.
    actionable: BitBoard,
    actionable_minors: BitBoard,
    actionable_rooks: BitBoard,
    actionable_pawns: BitBoard,
}

impl ColourScan {
    /// Attack units this colour brings against the enemy king zone.
    pub(super) fn attack_units(&self) -> i32 {
        self.zone_hits + self.attacker_units
    }
}

/// The board facts one colour's scan reads, gathered once per colour.
#[derive(Clone, Copy)]
struct ScanContext {
    color: Color,
    occupied: BitBoard,
    friendly_pieces: BitBoard,
    enemy_pieces: BitBoard,
    own_pawns: BitBoard,
    enemy_pawns: BitBoard,
    all_pawns: BitBoard,
    enemy_king: Square,
    enemy_king_zone: BitBoard,
    own_king_zone: BitBoard,
    seventh: Rank,
    eighth: Rank,
    space_mask: BitBoard,
    passer_spans: &'static [BitBoard; 64],
    challenges: &'static [BitBoard; 64],
    outpost_ranks: BitBoard,
    /// Every square an enemy pawn attacks, two shifts computed once.
    enemy_pawn_attacks: BitBoard,
    own_king: Square,
    pinned: BitBoard,
}

/// Scans one colour's pieces of one type.
///
/// The piece type and whether style terms are wanted are const parameters,
/// so each of the twelve instantiations is a loop containing only the work
/// that type and that path need. The objective path, which is what search
/// evaluates with, compiles with no style code in its loops at all. This
/// replaced a single generic loop over all six types whose body had grown,
/// term by term, past what the register allocator could keep in registers;
/// every addition was cheap and the loop as a whole had doubled in cost.
#[inline(always)]
fn scan_pieces<const PIECE: usize, const STYLE: bool>(
    board: &Board,
    context: &ScanContext,
    scan: &mut ColourScan,
    attack_counts: &mut [u8; 64],
) {
    let piece = Piece::ALL[PIECE];
    let color = context.color;
    let curve = weights::mobility_curve_offset(piece);
    let type_slot = curve.map(|_| PIECE - 1);
    let units = king_attack_units(piece);
    let is_pawn = piece == Piece::Pawn;
    let is_minor = matches!(piece, Piece::Knight | Piece::Bishop);
    let is_bishop = piece == Piece::Bishop;
    let is_rook = piece == Piece::Rook;
    let is_king = piece == Piece::King;
    let pressure_weight = match piece {
        Piece::Pawn => 3,
        Piece::Knight | Piece::Bishop => 4,
        Piece::Rook => 3,
        Piece::Queen => 2,
        Piece::King => 0,
    };

    for square in board.colored_pieces(color, piece) {
        // Placement and activity are accumulated in the same pass rather than
        // in a second loop over every piece: the terms differ but the
        // iteration is identical.
        scan.placement = scan.placement + placement::placement(piece, square, color);
        scan.activity += match piece {
            Piece::Knight | Piece::Bishop | Piece::Rook | Piece::Queen => centrality(square),
            Piece::Pawn => {
                let rank = square.rank() as i32;
                if color == Color::White {
                    (rank - 1).max(0)
                } else {
                    (6 - rank).max(0)
                }
            }
            Piece::King => 0,
        };
        let raw_attacks = attacks_from(piece, square, color, context.occupied);
        let attacks = raw_attacks & !context.friendly_pieces;
        let actionable =
            pin_restricted_attacks(raw_attacks, square, context.own_king, context.pinned);
        let mobility_attacks = actionable & !context.friendly_pieces;
        let moves = mobility_attacks.len() as i32;
        scan.mobility += moves;
        scan.piece_mobility[PIECE] += moves;
        if let Some(slot) = type_slot {
            // Moves onto squares an enemy pawn attacks, counted beside the
            // raw count rather than removed from it, so the mobility curve
            // and the unsafe-square penalty remain separate features.
            scan.unsafe_mobility[slot] +=
                (mobility_attacks & context.enemy_pawn_attacks).len() as i32;
            // Distance to the enemy king, bucketed at one, two, three and
            // four or more, weighted here as the mobility curves are.
            let bucket = king_distance(square, context.enemy_king).min(4) - 1;
            scan.tropism = scan.tropism + weights::tropism_weight(slot, bucket);
        }
        // Weighted here for the same reason placement is: the curve is a
        // table lookup per piece, and expanding it into one count per move
        // count is work only the fitter needs.
        if let Some(offset) = curve {
            scan.mobility_curves =
                scan.mobility_curves + weights::mobility_curve_at(offset + moves as usize);
        }
        if is_pawn && (context.enemy_pawns & context.passer_spans[square as usize]).is_empty() {
            scan.passers |= square.bitboard();
        }
        if is_minor
            && scan.pawn_held.has(square)
            && (context.enemy_pawns & context.challenges[square as usize]).is_empty()
        {
            scan.outposts[usize::from(piece == Piece::Bishop)] += 1;
        }
        if is_bishop {
            // A bishop's own pawns on its colour are the ones in its way.
            let same_colour = if LIGHT_SQUARES.has(square) {
                LIGHT_SQUARES
            } else {
                !LIGHT_SQUARES
            };
            scan.bishop_pawns_on_colour += (context.own_pawns & same_colour).len() as i32;
        }
        if is_rook {
            let file = square.file().bitboard();
            if (context.all_pawns & file).is_empty() {
                scan.rook_files[0] += 1;
            } else if (context.own_pawns & file).is_empty() {
                scan.rook_files[1] += 1;
            }
            // A rook on the seventh earns its name against a king it confines
            // or pawns it attacks along the rank, not for the square alone.
            if square.rank() == context.seventh
                && (context.enemy_king.rank() == context.eighth
                    || !(context.enemy_pawns & context.seventh.bitboard()).is_empty())
            {
                scan.rook_files[2] += 1;
            }
        }
        if !is_king {
            scan.actionable |= actionable;
            if is_pawn {
                scan.actionable_pawns |= actionable;
            } else if is_minor {
                scan.actionable_minors |= actionable;
            } else if is_rook {
                scan.actionable_rooks |= actionable;
            }
            scan.attacked_twice |= scan.attacked & raw_attacks;
            scan.attacked |= raw_attacks;
            if let Some(slot) = type_slot {
                scan.type_attacks[slot] |= raw_attacks;
            } else if is_pawn {
                scan.pawn_attacks |= raw_attacks;
            }
            // Branchless: the type units count once for any piece touching
            // the zone, the squares count each.
            let zone_squares = (attacks & context.enemy_king_zone).len() as i32;
            scan.zone_hits += zone_squares;
            scan.attacker_units += units * i32::from(zone_squares > 0);
        }
        if !STYLE {
            continue;
        }
        if !is_king {
            for target in raw_attacks {
                attack_counts[target as usize] += 1;
            }
            scan.zone_defenders += i32::from(!(raw_attacks & context.own_king_zone).is_empty());
        }

        let zone_hits = (attacks & context.enemy_king_zone).len() as i32;
        if zone_hits > 0 && !is_king {
            scan.profile.attackers += 1;
            scan.attacker_mask |= 1 << piece_index(piece);
            scan.profile.king_pressure += zone_hits * pressure_weight;
            if matches!(piece, Piece::Bishop | Piece::Rook | Piece::Queen) {
                scan.profile.open_lines += 1;
            }
        }

        if STYLE && !is_king && piece != Piece::Queen {
            for target in attacks & context.enemy_pieces {
                let Some(target_piece) = board.piece_on(target) else {
                    continue;
                };
                if target_piece != Piece::King && piece_value(piece) < piece_value(target_piece) {
                    scan.profile.threats +=
                        1 + (piece_value(target_piece) - piece_value(piece)) / 100;
                }
            }
        }

        if STYLE {
            scan.profile.space += (attacks & context.space_mask).len() as i32;
        }
    }
}

/// Scans one colour's pawns for the objective path, set-wise.
///
/// Pawns are half the pieces and every one of them attacks at most two
/// squares, one to each side, so everything the generic scan does per pawn
/// except the placement lookup and the passer test can be done for all of
/// them at once: the two attack sets are two shifts, a pawn's mobility is
/// the size of its attack set outside friendly pieces, a square two pawns
/// attack is one both shifts reach, and the pawns bearing on the king zone
/// are the pawns the zone shifts back onto. The result is identical to the
/// per-pawn scan, which the styled path still runs because its per-square
/// attack counts need every pawn walked; a test holds the two equal.
#[inline(always)]
fn scan_pawns(context: &ScanContext, scan: &mut ColourScan) {
    let color = context.color;
    let pawns = context.own_pawns;
    let not_a = !File::A.bitboard();
    let not_h = !File::H.bitboard();
    let (left, right, zone_left, zone_right, ranks): (BitBoard, BitBoard, BitBoard, BitBoard, _) =
        if color == Color::White {
            (
                BitBoard((pawns & not_a).0 << 7),
                BitBoard((pawns & not_h).0 << 9),
                BitBoard(context.enemy_king_zone.0 >> 7) & not_a,
                BitBoard(context.enemy_king_zone.0 >> 9) & not_h,
                [
                    Rank::Third,
                    Rank::Fourth,
                    Rank::Fifth,
                    Rank::Sixth,
                    Rank::Seventh,
                ],
            )
        } else {
            (
                BitBoard((pawns & not_a).0 >> 9),
                BitBoard((pawns & not_h).0 >> 7),
                BitBoard(context.enemy_king_zone.0 << 9) & not_a,
                BitBoard(context.enemy_king_zone.0 << 7) & not_h,
                [
                    Rank::Sixth,
                    Rank::Fifth,
                    Rank::Fourth,
                    Rank::Third,
                    Rank::Second,
                ],
            )
        };

    for square in pawns {
        scan.placement = scan.placement + placement::placement(Piece::Pawn, square, color);
        if (context.enemy_pawns & context.passer_spans[square as usize]).is_empty() {
            scan.passers |= square.bitboard();
        }
    }
    // Activity counts a pawn's advance from its second rank: one on the
    // third through five on the seventh.
    for (advance, rank) in ranks.into_iter().enumerate() {
        scan.activity += (advance as i32 + 1) * (pawns & rank.bitboard()).len() as i32;
    }

    let free = !context.friendly_pieces;
    let moves = (left & free).len() as i32 + (right & free).len() as i32;
    scan.mobility += moves;
    scan.piece_mobility[0] += moves;

    scan.attacked_twice |= scan.attacked & (left | right) | (left & right);
    scan.attacked |= left | right;
    scan.pawn_attacks |= left | right;
    scan.actionable |= left | right;
    scan.actionable_pawns |= left | right;

    let zone = context.enemy_king_zone & free;
    scan.zone_hits += (left & zone).len() as i32 + (right & zone).len() as i32;
    let bearing = pawns & (zone_left | zone_right);
    scan.attacker_units += king_attack_units(Piece::Pawn) * bearing.len() as i32;
}

/// Scans every piece of one colour, one specialised loop per type.
#[inline(always)]
fn scan_colour<const STYLE: bool>(
    board: &Board,
    context: &ScanContext,
    scan: &mut ColourScan,
    attack_counts: &mut [u8; 64],
) {
    if STYLE || !(context.pinned & context.own_pawns).is_empty() {
        scan_pieces::<0, STYLE>(board, context, scan, attack_counts);
    } else {
        scan_pawns(context, scan);
    }
    // The pawn scan has gathered every square a pawn attacks, which is the
    // outpost support the minor scans need.
    scan.pawn_held = scan.pawn_attacks & context.outpost_ranks;
    scan_pieces::<1, STYLE>(board, context, scan, attack_counts);
    scan_pieces::<2, STYLE>(board, context, scan, attack_counts);
    scan_pieces::<3, STYLE>(board, context, scan, attack_counts);
    scan_pieces::<4, STYLE>(board, context, scan, attack_counts);
    scan_pieces::<5, STYLE>(board, context, scan, attack_counts);
}

pub(super) fn attack_summary_with_style(board: &Board, style: bool) -> AttackSummary {
    let occupied = board.occupied();
    let king_zones = [
        get_king_moves(board.king(Color::White)) | board.colored_pieces(Color::White, Piece::King),
        get_king_moves(board.king(Color::Black)) | board.colored_pieces(Color::Black, Piece::King),
    ];
    let all_pawns = board.pieces(Piece::Pawn);
    let mut summary = AttackSummary::default();
    let mut attack_counts = [[0_u8; 64]; 2];

    for color in [Color::White, Color::Black] {
        let index = color as usize;
        let enemy = !color;
        let enemy_king = board.king(enemy);
        let own_pawns = board.colored_pieces(color, Piece::Pawn);
        let enemy_pawns = board.colored_pieces(enemy, Piece::Pawn);
        let (seventh, eighth) = if color == Color::White {
            (Rank::Seventh, Rank::Eighth)
        } else {
            (Rank::Second, Rank::First)
        };
        let passer_spans = if color == Color::White {
            &WHITE_PASSER_SPANS
        } else {
            &BLACK_PASSER_SPANS
        };
        // Outposts are on the owner's fourth to sixth ranks, defended by a
        // pawn, and beyond the reach of every enemy pawn.
        let (outpost_ranks, challenges) = if color == Color::White {
            (
                Rank::Fourth.bitboard() | Rank::Fifth.bitboard() | Rank::Sixth.bitboard(),
                &WHITE_OUTPOST_CHALLENGES,
            )
        } else {
            (
                Rank::Fifth.bitboard() | Rank::Fourth.bitboard() | Rank::Third.bitboard(),
                &BLACK_OUTPOST_CHALLENGES,
            )
        };
        let context = ScanContext {
            color,
            occupied,
            friendly_pieces: board.colors(color),
            enemy_pieces: board.colors(enemy),
            own_pawns,
            enemy_pawns,
            all_pawns,
            enemy_king,
            enemy_king_zone: king_zones[enemy as usize],
            own_king_zone: king_zones[index],
            seventh,
            eighth,
            space_mask: if color == Color::White {
                BitBoard(0xFFFF_FFFF_0000_0000)
            } else {
                BitBoard(0x0000_0000_FFFF_FFFF)
            },
            passer_spans,
            challenges,
            outpost_ranks,
            enemy_pawn_attacks: pawn_attack_set(enemy_pawns, enemy),
            own_king: board.king(color),
            pinned: pinned_pieces(board, color),
        };
        let scan = &mut summary.scans[index];
        if style {
            scan_colour::<true>(board, &context, scan, &mut attack_counts[index]);
        } else {
            scan_colour::<false>(board, &context, scan, &mut attack_counts[index]);
        }

        // A blockaded passer is one with any piece on the square ahead. The
        // passer test is a function of the pawns, but the blocker is a piece,
        // which is why this is here and not in the cache. Shifting the
        // occupancy back one rank puts every blocker on its passer's square,
        // so the blocked set is one intersection and is usually empty.
        let blockers = if color == Color::White {
            BitBoard(occupied.0 >> 8)
        } else {
            BitBoard(occupied.0 << 8)
        };
        for square in scan.passers & blockers {
            let advance = if color == Color::White {
                square.rank() as usize
            } else {
                7 - square.rank() as usize
            };
            summary.blocked_passers[index][advance - 1] += 1;
        }

        scan.profile.attacker_variety = scan.attacker_mask.count_ones() as i32;
        scan.profile.king_pressure += scan.profile.attackers * scan.profile.attackers * 2;
        if style {
            let king_file = enemy_king.file() as i32;
            let king_rank = enemy_king.rank() as i32;
            for pawn in own_pawns {
                if (pawn.file() as i32 - king_file).abs() <= 1 {
                    let distance = if color == Color::White {
                        king_rank - pawn.rank() as i32
                    } else {
                        pawn.rank() as i32 - king_rank
                    };
                    if (1..=4).contains(&distance) {
                        scan.profile.pawn_storm += 5 - distance;
                    }
                }
                scan.profile.pawn_breaks += (get_pawn_attacks(pawn, color) & enemy_pawns)
                    .into_iter()
                    .filter(|target| (target.file() as i32 - king_file).abs() <= 1)
                    .count() as i32;
            }
        }
    }

    if style {
        for color in [Color::White, Color::Black] {
            let index = color as usize;
            let enemy = !color;
            let defenders = summary.scans[enemy as usize].zone_defenders;
            let result = &mut summary.scans[index].profile;
            result.defender_shortage = (result.attackers - defenders).max(0);
            for target in board.colors(enemy) & !board.pieces(Piece::King) {
                let attackers = i32::from(attack_counts[index][target as usize]);
                if attackers >= 2 {
                    if let Some(target_piece) = board.piece_on(target) {
                        result.supported_threats +=
                            (attackers - 1) * (1 + piece_value(target_piece) / 300);
                    }
                }
            }
        }
    }

    // Combine both geometric scans for threats and passer paths. Direct-check
    // safety is tested separately with the checking origin vacated.
    for color in [Color::White, Color::Black] {
        let index = color as usize;
        let enemy = !color;
        let enemy_king = board.king(enemy);
        let king_reach = get_king_moves(enemy_king);
        let own = &summary.scans[index];
        let theirs = &summary.scans[enemy as usize];

        // Threats against the enemy's pieces, kings excluded on both sides of
        // the ledger: a minor attacked by a pawn, a piece attacked and left
        // undefended, and a piece attacked by something worth less than it.
        let enemy_pieces =
            board.colors(enemy) & !board.pieces(Piece::Pawn) & !board.pieces(Piece::King);
        let enemy_minors =
            enemy_pieces & (board.pieces(Piece::Knight) | board.pieces(Piece::Bishop));
        let enemy_majors = enemy_pieces & (board.pieces(Piece::Rook) | board.pieces(Piece::Queen));
        let enemy_queens = enemy_pieces & board.pieces(Piece::Queen);
        let own_reach = own.attacked | get_king_moves(board.king(color));
        let enemy_defends = theirs.attacked | king_reach;

        // Passers by the state of their path: the file ahead of each, which
        // the passer span holds, against the enemy's reach with the king's
        // moves put back, against the occupancy, and along the file behind
        // for a friendly rook with nothing between.
        let spans = if color == Color::White {
            &WHITE_PASSER_SPANS
        } else {
            &BLACK_PASSER_SPANS
        };
        let own_rooks = board.colored_pieces(color, Piece::Rook);
        for passer in own.passers {
            let file = passer.file().bitboard();
            let path = spans[passer as usize] & file;
            let advance = if color == Color::White {
                passer.rank() as usize
            } else {
                7 - passer.rank() as usize
            };
            if (path & enemy_defends).is_empty() {
                summary.passer_safe_path[index][advance - 1] += 1;
                summary.passer_path[index] =
                    summary.passer_path[index] + weights::passer_safe_path_weight(advance - 1);
            }
            if (path & occupied).is_empty() {
                summary.passer_free_path[index][advance - 1] += 1;
                summary.passer_path[index] =
                    summary.passer_path[index] + weights::passer_free_path_weight(advance - 1);
            }
            let behind = get_rook_moves(passer, occupied) & file & !path;
            summary.rook_behind_passer[index] += i32::from(!(behind & own_rooks).is_empty());
        }
        let minor_attacks = own.actionable_minors;
        let threat_reach = own.actionable | get_king_moves(board.king(color));
        summary.threats[index][0] = (enemy_minors & own.actionable_pawns).len() as i32;
        summary.threats[index][1] = (enemy_pieces & threat_reach & !enemy_defends).len() as i32;
        summary.threats[index][2] = (enemy_majors & (own.actionable_pawns | minor_attacks)).len()
            as i32
            + (enemy_queens & own.actionable_rooks).len() as i32;

        // A pawn that can push, once or twice from its second rank, onto an
        // empty square no enemy pawn attacks that is either unattacked or
        // defended, and from there attack an enemy piece, is a threat the
        // side has not made yet.
        let own_pawns = board.colored_pieces(color, Piece::Pawn);
        let empty = !occupied;
        let pushes = if color == Color::White {
            let single = BitBoard(own_pawns.0 << 8) & empty;
            single | (BitBoard((single & Rank::Third.bitboard()).0 << 8) & empty)
        } else {
            let single = BitBoard(own_pawns.0 >> 8) & empty;
            single | (BitBoard((single & Rank::Sixth.bitboard()).0 >> 8) & empty)
        };
        let safe_pushes = pushes & !theirs.pawn_attacks & (!enemy_defends | own_reach);
        summary.pawn_push_threats[index] =
            (pawn_attack_set(safe_pushes, color) & enemy_pieces).len() as i32;

        summary.safe_checks[index] = safe_check_counts(board, color, own, theirs);
    }

    summary
}

/// Geometric attackers under a hypothetical occupancy, excluding removed pieces.
/// Pinned pieces still control squares against kings.
#[inline(always)]
fn geometric_attackers_to(
    board: &Board,
    color: Color,
    target: Square,
    occupied: BitBoard,
    removed: BitBoard,
) -> BitBoard {
    let available = board.colors(color) & !removed;
    let queens = board.pieces(Piece::Queen);
    ((get_pawn_attacks(target, !color) & board.pieces(Piece::Pawn))
        | (get_knight_moves(target) & board.pieces(Piece::Knight))
        | (get_bishop_moves(target, occupied) & (board.pieces(Piece::Bishop) | queens))
        | (get_rook_moves(target, occupied) & (board.pieces(Piece::Rook) | queens))
        | (get_king_moves(target) & board.pieces(Piece::King)))
        & available
}

/// Validate the fitted N/B/R checking candidates against immediate captures.
/// The queen slot retains its geometric approximation. Destinations count once
/// per piece type, even if several origins can use them.
fn safe_check_counts(
    board: &Board,
    color: Color,
    own: &ColourScan,
    theirs: &ColourScan,
) -> [i32; 4] {
    let candidates = safe_check_candidates(board, color, own, theirs);
    let mut counts = [0, 0, 0, candidates[3].len() as i32];
    if candidates[..3].iter().all(|squares| squares.is_empty()) {
        return counts;
    }
    let own_king = board.king(color);
    let enemy_king = board.king(!color);
    let in_check = theirs.attacked.has(own_king) || get_king_moves(enemy_king).has(own_king);
    let pinned = if board.side_to_move() == color {
        board.pinned() & board.colors(color)
    } else {
        pinned_pieces(board, color)
    };
    for (slot, piece) in [Piece::Knight, Piece::Bishop, Piece::Rook]
        .into_iter()
        .enumerate()
    {
        counts[slot] =
            safe_check_destinations(board, color, piece, candidates[slot], pinned, in_check).len()
                as i32;
    }
    counts
}

/// Preserve the fitted feature's candidate scope before validating occupancy.
/// Newly opened support and pinned defenders may leave safe checks uncounted.
fn safe_check_candidates(
    board: &Board,
    color: Color,
    own: &ColourScan,
    theirs: &ColourScan,
) -> [BitBoard; 4] {
    let enemy_king = board.king(!color);
    let occupied = board.occupied();
    let diagonals = get_bishop_moves(enemy_king, occupied);
    let lines = get_rook_moves(enemy_king, occupied);
    let checks = [
        get_knight_moves(enemy_king),
        diagonals,
        lines,
        diagonals | lines,
    ];
    let safe = !theirs.attacked & (!get_king_moves(enemy_king) | own.attacked_twice);
    let landing = !board.colors(color) & !enemy_king.bitboard() & safe;
    std::array::from_fn(|slot| own.type_attacks[slot] & checks[slot] & landing)
}

fn safe_check_destinations(
    board: &Board,
    color: Color,
    piece: Piece,
    candidates: BitBoard,
    pinned: BitBoard,
    in_check: bool,
) -> BitBoard {
    if candidates.is_empty() {
        return BitBoard::EMPTY;
    }
    let occupied = board.occupied();
    let own_king = board.king(color);
    let enemy = !color;
    let enemy_king = board.king(enemy);
    let mut safe = BitBoard::EMPTY;
    for from in board.colored_pieces(color, piece) {
        let attacks = attacks_from(piece, from, color, occupied);
        let destinations =
            pin_restricted_attacks(attacks, from, own_king, pinned) & candidates & !safe;
        for to in destinations {
            let after_move = (occupied & !from.bitboard()) | to.bitboard();
            if !attacks_from(piece, to, color, after_move).has(enemy_king) {
                continue;
            }
            // Outside check, the pin-line restriction proves king safety for
            // these ordinary non-king moves. Check evasions need a fresh probe.
            if in_check
                && !geometric_attackers_to(board, enemy, own_king, after_move, to.bitboard())
                    .is_empty()
            {
                continue;
            }
            let capturers = geometric_attackers_to(board, enemy, to, after_move, to.bitboard());
            let capturable = capturers.into_iter().any(|capturer| {
                let after_capture = after_move & !capturer.bitboard();
                let king = if capturer == enemy_king {
                    to
                } else {
                    enemy_king
                };
                // Once the checker is captured, the only friendly piece
                // removed from the original masks is the checking origin.
                geometric_attackers_to(board, color, king, after_capture, from.bitboard())
                    .is_empty()
            });
            if !capturable {
                safe |= to.bitboard();
            }
        }
    }
    safe
}

/// Returns how many squares a piece may move to, for the fitter's expansion.
#[cfg(feature = "tuning")]
pub(super) fn mobility_count(board: &Board, piece: Piece, square: Square, color: Color) -> usize {
    let attacks = attacks_from(piece, square, color, board.occupied());
    (pin_restricted_attacks(
        attacks,
        square,
        board.king(color),
        pinned_pieces(board, color),
    ) & !board.colors(color))
    .len() as usize
}

/// Counts each side's knights, bishops, rooks and queens by their bucketed
/// distance to the enemy king, White-positive, as the fitter's expansion of
/// the pair the piece loop carries.
#[cfg(any(test, feature = "tuning"))]
pub(super) fn tropism_counts(board: &Board) -> [i32; 16] {
    let mut counts = [0_i32; 16];
    for color in [Color::White, Color::Black] {
        let sign = if color == Color::White { 1 } else { -1 };
        let enemy_king = board.king(!color);
        for (slot, piece) in [Piece::Knight, Piece::Bishop, Piece::Rook, Piece::Queen]
            .into_iter()
            .enumerate()
        {
            for square in board.colored_pieces(color, piece) {
                let bucket = king_distance(square, enemy_king).min(4) - 1;
                counts[slot * 4 + bucket] += sign;
            }
        }
    }
    counts
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

#[inline(always)]
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

#[inline(always)]
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PawnFeatures {
    doubled: i32,
    isolated: i32,
    /// Pawns that cannot be supported by a neighbour and cannot safely
    /// advance: no friendly pawn stands level or behind on an adjacent file,
    /// and the square in front is attacked or occupied by an enemy pawn.
    backward: i32,
    passed_by_rank: [i8; 6],
    protected_passer_by_rank: [i8; 6],
    /// Pawns with a neighbour beside them or defending them, by rank.
    connected_by_rank: [i8; 6],
    /// Pawns not passed whose own file ahead is clear and whose helpers on
    /// the adjacent files can match the enemy pawns ahead there, by rank.
    candidate_by_rank: [i8; 6],
    /// Runs of adjacent files holding a pawn.
    islands: i32,
}

/// Reference pawn structure, retained to check the mask-based extraction.
///
/// This is the scanning form the masks replaced: for each pawn it walks every
/// enemy pawn looking for one ahead on its own or an adjacent file.
#[cfg(test)]
fn reference_pawn_features(board: &Board, color: Color) -> PawnFeatures {
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
        if count > 0 && (file == 0 || files[file - 1] == 0) {
            result.islands += 1;
        }
    }

    for square in pawns {
        let file = square.file() as i32;
        let rank = square.rank() as i32;
        let forward = if color == Color::White { 1 } else { -1 };
        let advance = if color == Color::White {
            rank
        } else {
            7 - rank
        };
        let index = (advance - 1) as usize;

        let neighbour = |other: Square, rank_delta: i32| {
            (other.file() as i32 - file).abs() == 1
                && (other.rank() as i32 - rank) * forward == rank_delta
        };
        let phalanx = pawns.into_iter().any(|other| neighbour(other, 0));
        let supported = pawns.into_iter().any(|other| neighbour(other, -1));
        if phalanx || supported {
            result.connected_by_rank[index] += 1;
        }

        let supportable = pawns.into_iter().any(|other| {
            (other.file() as i32 - file).abs() == 1 && (other.rank() as i32 - rank) * forward <= 0
        });
        let stop_rank = rank + forward;
        let stop_unsafe = enemy_pawns.into_iter().any(|enemy| {
            let enemy_file = enemy.file() as i32;
            let enemy_rank = enemy.rank() as i32;
            (enemy_file == file && enemy_rank == stop_rank)
                || ((enemy_file - file).abs() == 1 && enemy_rank == stop_rank + forward)
        });
        if !supportable && stop_unsafe {
            result.backward += 1;
        }

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
            result.passed_by_rank[index] += 1;
            if !(get_pawn_attacks(square, !color) & pawns).is_empty() {
                result.protected_passer_by_rank[index] += 1;
            }
        } else {
            let ahead = |other: Square| (other.rank() as i32 - rank) * forward > 0;
            let file_clear = !enemy_pawns
                .into_iter()
                .any(|enemy| enemy.file() as i32 == file && ahead(enemy));
            let sentries = enemy_pawns
                .into_iter()
                .filter(|&enemy| (enemy.file() as i32 - file).abs() == 1 && ahead(enemy))
                .count();
            let helpers = pawns
                .into_iter()
                .filter(|&other| (other.file() as i32 - file).abs() == 1 && !ahead(other))
                .count();
            if file_clear && helpers >= sentries {
                result.candidate_by_rank[index] += 1;
            }
        }
    }

    result
}

/// Reference king safety, retained to check the mask-based extraction.
#[cfg(test)]
fn reference_king_safety(board: &Board, color: Color) -> (i32, i32) {
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

fn pawn_features(board: &Board, color: Color) -> PawnFeatures {
    let pawns = board.colored_pieces(color, Piece::Pawn);
    let enemy_pawns = board.colored_pieces(!color, Piece::Pawn);
    let spans = if color == Color::White {
        &WHITE_PASSER_SPANS
    } else {
        &BLACK_PASSER_SPANS
    };
    let (challenges, forward) = if color == Color::White {
        (&WHITE_OUTPOST_CHALLENGES, 1)
    } else {
        (&BLACK_OUTPOST_CHALLENGES, -1)
    };
    let enemy_attacks = pawn_attack_set(enemy_pawns, !color);

    let mut result = PawnFeatures::default();
    let mut occupied_files = 0_u8;
    for file in File::ALL {
        let count = (pawns & file.bitboard()).len();
        result.doubled += count.saturating_sub(1) as i32;
        if count > 0 && (pawns & file.adjacent()).is_empty() {
            result.isolated += count as i32;
        }
        occupied_files |= u8::from(count > 0) << (file as u8);
    }
    // An island starts at every occupied file whose lower neighbour is
    // empty, which one shift and one mask count for all eight files.
    result.islands = (occupied_files & !(occupied_files << 1)).count_ones() as i32;

    for square in pawns {
        let rank = square.rank() as i32;
        let advance = if color == Color::White {
            rank
        } else {
            7 - rank
        };
        // A pawn never stands on its own first or last rank, so `advance` is
        // always one to six and the index is always in bounds.
        let index = (advance - 1) as usize;
        let adjacent = square.file().adjacent();
        // A pawn defends the squares an enemy pawn on this square would
        // attack, so intersecting those with our own pawns answers whether
        // this pawn is supported. This stays a pure function of the two pawn
        // sets, which is what lets it ride the structure cache.
        let supported = !(get_pawn_attacks(square, !color) & pawns).is_empty();
        let phalanx = !(pawns & adjacent & square.rank().bitboard()).is_empty();
        if supported || phalanx {
            result.connected_by_rank[index] += 1;
        }

        // Backward: nothing level or behind on an adjacent file can ever
        // support it, and the square in front is not safe to step onto. The
        // level-or-behind mask is the adjacent files less the part ahead,
        // which the outpost table already holds.
        let supportable = !(pawns & adjacent & !challenges[square as usize]).is_empty();
        let stop = square.try_offset(0, forward);
        if !supportable && stop.is_some_and(|stop| enemy_attacks.has(stop) || enemy_pawns.has(stop))
        {
            result.backward += 1;
        }

        // A pawn is passed when no enemy pawn stands ahead of it on its own or
        // an adjacent file, which the precomputed span answers in one test.
        if (enemy_pawns & spans[square as usize]).is_empty() {
            result.passed_by_rank[index] += 1;
            if supported {
                result.protected_passer_by_rank[index] += 1;
            }
        } else if (enemy_pawns & spans[square as usize] & square.file().bitboard()).is_empty() {
            // Not passed, but nothing stands in its own file's way: a
            // candidate when the friendly pawns level with or behind it on
            // the adjacent files can match the enemy pawns ahead on them.
            let helpers = (pawns & adjacent & !challenges[square as usize]).len();
            let sentries = (enemy_pawns & challenges[square as usize]).len();
            if helpers >= sentries {
                result.candidate_by_rank[index] += 1;
            }
        }
    }

    result
}

fn king_safety(board: &Board, color: Color) -> (i32, i32) {
    let king = board.king(color);
    let pawns = board.colored_pieces(color, Piece::Pawn);
    let zones = if color == Color::White {
        &WHITE_SHELTER_ZONES
    } else {
        &BLACK_SHELTER_ZONES
    };
    // Shelter counts friendly pawns one or two ranks ahead of the king on its own
    // or an adjacent file, which the precomputed zone answers in one test.
    let shelter = (pawns & zones[king as usize]).len() as i32;

    // An open king file is one of the king's own or adjacent files carrying no
    // friendly pawn. Files off the edge of the board are not counted, so a king
    // on a rim file has two neighbouring files rather than three.
    let mut open_files = 0;
    for file in File::ALL {
        if KING_FILE_SPANS[king as usize].has(Square::new(file, Rank::First))
            && (pawns & file.bitboard()).is_empty()
        {
            open_files += 1;
        }
    }

    (shelter, open_files)
}

/// Grades the king's shelter by where its nearest pawn stands on each file.
///
/// The shelter count says how many pawns stand in a box ahead of the king;
/// it cannot say that an unmoved g2 and h2 are a different shelter from a
/// g3 and h4 that have been lured forward. For the king's own file and each
/// adjacent file on the board, the nearest friendly pawn ahead is found and
/// counted by its rank distance, one to six. A file with no pawn ahead is
/// the open-file term's business and counts nowhere here. The three files
/// ahead of the king are the passer span, so no new table is needed.
fn shelter_distances(board: &Board, color: Color) -> ([i8; 6], [i8; 6]) {
    let king = board.king(color);
    let pawns = board.colored_pieces(color, Piece::Pawn);
    let spans = if color == Color::White {
        &WHITE_PASSER_SPANS
    } else {
        &BLACK_PASSER_SPANS
    };
    let ahead = pawns & spans[king as usize];
    let mut king_file = [0_i8; 6];
    let mut adjacent = [0_i8; 6];
    let king_rank = king.rank() as u32;
    // The nearest pawn on a file is its lowest set square for White and its
    // highest for Black, which the bit scans answer without iterating pawns.
    for file in File::ALL {
        let file_pawns = ahead & file.bitboard();
        if file_pawns.is_empty() {
            continue;
        }
        let nearest_rank = if color == Color::White {
            file_pawns.0.trailing_zeros() / 8
        } else {
            (63 - file_pawns.0.leading_zeros()) / 8
        };
        let distance = nearest_rank.abs_diff(king_rank) as usize;
        let counts = if file == king.file() {
            &mut king_file
        } else {
            &mut adjacent
        };
        counts[distance - 1] += 1;
    }
    (king_file, adjacent)
}

/// Grades the pawn storm against the king by the nearest enemy pawn's rank.
///
/// The shelter says where the king's own pawns stand; this says where the
/// enemy's are coming from. For the king's own file and each adjacent file
/// on the board, the nearest enemy pawn ahead of the king is found and
/// counted by its rank distance, one to six. A storming pawn that a friendly
/// pawn stands directly in front of is counted in a block of its own rather
/// than by file: a blocked storm is a closed file and an open one is a
/// lever, and the fit should be free to price the two apart. An enemy pawn
/// level with or behind the king has passed it and counts nowhere here.
/// Like the shelter, the term is a function of the pawns and the king
/// square, so it rides the structure cache.
fn storm_distances(board: &Board, color: Color) -> ([i8; 6], [i8; 6], [i8; 6]) {
    let king = board.king(color);
    let pawns = board.colored_pieces(color, Piece::Pawn);
    let enemy_pawns = board.colored_pieces(!color, Piece::Pawn);
    let spans = if color == Color::White {
        &WHITE_PASSER_SPANS
    } else {
        &BLACK_PASSER_SPANS
    };
    let ahead = enemy_pawns & spans[king as usize];
    // A friendly pawn shifted one rank toward the enemy lands on the
    // storming pawn it blocks, so the blocked set is one intersection.
    let blockers = if color == Color::White {
        BitBoard(pawns.0 << 8)
    } else {
        BitBoard(pawns.0 >> 8)
    };
    let mut king_file = [0_i8; 6];
    let mut adjacent = [0_i8; 6];
    let mut blocked = [0_i8; 6];
    let king_rank = king.rank() as u32;
    for file in File::ALL {
        let file_pawns = ahead & file.bitboard();
        if file_pawns.is_empty() {
            continue;
        }
        // The nearest enemy pawn on a file is its lowest set square for a
        // White king and its highest for a Black one, as for the shelter.
        let nearest = if color == Color::White {
            file_pawns.0.trailing_zeros()
        } else {
            63 - file_pawns.0.leading_zeros()
        };
        let distance = (nearest / 8).abs_diff(king_rank) as usize;
        let counts = if blockers.0 & (1_u64 << nearest) != 0 {
            &mut blocked
        } else if file == king.file() {
            &mut king_file
        } else {
            &mut adjacent
        };
        counts[distance - 1] += 1;
    }
    (king_file, adjacent, blocked)
}

/// Whether the king's flank holds no pawn of either colour.
///
/// The flank is the four files nearest the king. With no pawns on it the
/// king has no shelter to rebuild and the attacker no storm to pay for,
/// which is a different position from the one the shelter and storm terms
/// describe between them as merely open.
fn pawnless_flank(board: &Board, color: Color) -> i32 {
    let flank = KING_FLANKS[board.king(color).file() as usize];
    i32::from((board.pieces(Piece::Pawn) & flank).is_empty())
}

/// Counts the safe centre squares behind a colour's pawns.
///
/// The space a side controls is the centre-file squares on its own second
/// to fourth ranks that no enemy pawn attacks and no friendly pawn stands
/// on, and a square with a friendly pawn one or two ranks ahead of it counts
/// twice, because it is room the pawn chain has fenced off rather than
/// merely reached. This is the count alone; what the room is worth depends
/// on how many pieces want it, which the extraction multiplies in because
/// that product is not a function of the pawns. The whole count is, so it
/// rides the structure cache.
fn space_area(board: &Board, color: Color) -> i8 {
    let pawns = board.colored_pieces(color, Piece::Pawn);
    let enemy_attacks = pawn_attack_set(board.colored_pieces(!color, Piece::Pawn), !color);
    let (zone, behind) = if color == Color::White {
        (WHITE_SPACE_ZONE, BitBoard(pawns.0 >> 8 | pawns.0 >> 16))
    } else {
        (BLACK_SPACE_ZONE, BitBoard(pawns.0 << 8 | pawns.0 << 16))
    };
    let safe = zone & !pawns & !enemy_attacks;
    (safe.len() + (safe & behind).len()) as i8
}

/// Every square a colour's pawns attack, as two shifts.
#[inline(always)]
fn pawn_attack_set(pawns: BitBoard, color: Color) -> BitBoard {
    let not_a = (pawns & !File::A.bitboard()).0;
    let not_h = (pawns & !File::H.bitboard()).0;
    if color == Color::White {
        BitBoard((not_a << 7) | (not_h << 9))
    } else {
        BitBoard((not_a >> 9) | (not_h >> 7))
    }
}

#[cfg(test)]
mod tests {
    use cozy_chess::{Board, Color, Move};

    use super::{attack_summary, reference_attacking_features, reference_mobility};

    fn assert_matches_reference(board: &Board) {
        let cached = attack_summary(board);
        for color in [Color::White, Color::Black] {
            assert_eq!(
                cached.scans[color as usize].profile,
                reference_attacking_features(board, color),
                "attack features differ for {color:?} in {board}"
            );
            assert_eq!(
                cached.scans[color as usize].mobility,
                reference_mobility(board, color),
                "mobility differs for {color:?} in {board}"
            );
            assert_eq!(
                cached.scans[color as usize].activity,
                super::activity(board, color),
                "activity differs for {color:?} in {board}"
            );
            assert_eq!(
                super::pawn_features(board, color),
                super::reference_pawn_features(board, color),
                "pawn structure differs for {color:?} in {board}"
            );
            assert_eq!(
                super::king_safety(board, color),
                super::reference_king_safety(board, color),
                "king safety differs for {color:?} in {board}"
            );
        }
    }

    /// The structure cache must return what recomputation would produce.
    ///
    /// This is the property the cache rests on: it stores the full inputs and
    /// compares them, so a hit is exact rather than probabilistic. The walk visits
    /// many positions that collide into the same slots, which is what exercises
    /// eviction and mismatched keys rather than only fresh inserts.
    /// Structure terms must be signed from White's perspective.
    ///
    /// The differential test above compares the cache against recomputation, and
    /// both share this sign convention, so neither would notice if it inverted.
    /// This pins exact values rather than inequalities, so no comparison here can
    /// be loosened without failing.
    #[test]
    fn structure_terms_are_signed_from_whites_perspective() {
        // White has a doubled, isolated, passed a-file pair; Black has nothing.
        let white_weak: Board = "4k3/8/8/8/8/P7/P7/4K3 w - - 0 1".parse().unwrap();
        let terms = super::compute_structure_terms(&white_weak);
        assert_eq!(terms.doubled, 1);
        assert_eq!(terms.isolated, 2);

        // The mirror image inverts every term exactly.
        let black_weak: Board = "4k3/p7/p7/8/8/8/8/4K3 w - - 0 1".parse().unwrap();
        let mirrored = super::compute_structure_terms(&black_weak);
        assert_eq!(mirrored.doubled, -1);
        assert_eq!(mirrored.isolated, -2);

        // A passed pawn is signed by its owner. The pair is a true vertical
        // mirror, e5 against e4, so the rank they land on matches as well as
        // the sign: both stand four ranks from home, at index three.
        let white_passer: Board = "4k3/8/8/4P3/8/8/8/4K3 w - - 0 1".parse().unwrap();
        let black_passer: Board = "4k3/8/8/8/4p3/8/8/4K3 w - - 0 1".parse().unwrap();
        let white_passed = super::structure_counts(&white_passer).passed_by_rank;
        let black_passed = super::structure_counts(&black_passer).passed_by_rank;
        assert_eq!(white_passed, [0, 0, 0, 1, 0, 0]);
        assert_eq!(black_passed, [0, 0, 0, -1, 0, 0]);
    }

    /// A protected passer is counted only where a friendly pawn defends it.
    #[test]
    fn protected_passers_are_counted_separately_from_bare_ones() {
        // b5 and c6: the c-pawn is passed and defended by the b-pawn, and the
        // b-pawn is passed and defended by nothing.
        let board: Board = "4k3/8/2P5/1P6/8/8/8/4K3 w - - 0 1".parse().unwrap();
        let counts = super::structure_counts(&board);

        assert_eq!(counts.passed_by_rank, [0, 0, 0, 1, 1, 0]);
        assert_eq!(counts.protected_passer_by_rank, [0, 0, 0, 0, 1, 0]);
    }

    /// The style scalar must survive the change to per-rank counts unaltered.
    ///
    /// The attacking style weights passers by progress and is personality, so
    /// it is derived from the new counts rather than replaced. This checks the
    /// derivation against the sum the old scalar computed directly.
    #[test]
    fn the_derived_passer_scalar_matches_the_weighted_sum() {
        for fen in [
            "4k3/8/8/4P3/8/8/8/4K3 w - - 0 1",
            "4k3/8/2P5/1P6/8/8/8/4K3 w - - 0 1",
            "4k3/8/8/8/4p3/8/8/4K3 w - - 0 1",
            "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
        ] {
            let board: Board = fen.parse().unwrap();
            let features = super::extract(&board);
            let counts = super::structure_counts(&board);
            let expected: i32 = (0..6)
                .map(|rank| (rank as i32 + 1) * counts.passed_by_rank[rank])
                .sum();

            assert_eq!(features.passed_pawns, expected, "{fen}");
        }
    }

    #[test]
    fn cached_structure_matches_recomputation_over_a_playout() {
        let mut board = Board::default();
        let mut checked = 0_u32;
        for step in 0..160 {
            assert_eq!(
                super::structure_terms(&board),
                super::compute_structure_terms(&board),
                "structure cache disagreed after {step} plies in {board}",
            );
            checked += 1;
            let mut moves = Vec::new();
            board.generate_moves(|piece_moves| {
                moves.extend(piece_moves);
                false
            });
            if moves.is_empty() {
                break;
            }
            let chess_move = moves[step % moves.len()];
            board.play_unchecked(chess_move);
        }

        assert_eq!(checked, 160, "the playout should have compared every ply",);
    }

    /// Repeated queries for one position must agree with each other.
    #[test]
    fn cached_structure_is_stable_across_repeated_queries() {
        let board: Board = "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1"
            .parse()
            .unwrap();
        let expected = super::compute_structure_terms(&board);

        for _ in 0..8 {
            assert_eq!(super::structure_terms(&board), expected);
        }
    }

    /// Positions differing only in king square must not share a cache entry.
    #[test]
    fn structure_keys_separate_positions_that_differ_only_by_a_king() {
        let left: Board = "4k3/8/8/8/8/8/PPP5/K7 w - - 0 1".parse().unwrap();
        let right: Board = "4k3/8/8/8/8/8/PPP5/1K6 w - - 0 1".parse().unwrap();

        let left_key = super::StructureKey::new(&left);
        let right_key = super::StructureKey::new(&right);

        assert_ne!(left_key, right_key);
        assert_eq!(super::structure_terms(&left), {
            let _ = super::structure_terms(&right);
            super::compute_structure_terms(&left)
        });
    }

    #[test]
    fn pinned_knights_have_no_objective_mobility() {
        for (fen, color) in [
            ("k3r3/8/8/8/8/8/4N3/4K3 w - - 0 1", Color::White),
            ("4k3/4n3/8/8/8/8/8/K3R3 b - - 0 1", Color::Black),
        ] {
            let board: Board = fen.parse().unwrap();
            for style in [false, true] {
                let summary = super::attack_summary_with_style(&board, style);
                assert_eq!(summary.scans[color as usize].piece_mobility[1], 0, "{fen}");
            }
        }
    }

    #[test]
    fn pinned_knights_do_not_create_actionable_queen_threats() {
        for (fen, color) in [
            ("k3r3/8/8/8/5q2/8/4N3/4K3 w - - 0 1", Color::White),
            ("4k3/4n3/8/5Q2/8/8/8/K3R3 b - - 0 1", Color::Black),
        ] {
            let board: Board = fen.parse().unwrap();
            let summary = super::attack_summary(&board);
            assert_eq!(summary.threats[color as usize], [0; 3], "{fen}");
        }
    }

    #[test]
    fn pinned_sliders_and_pawns_keep_only_the_pin_line() {
        use cozy_chess::{Piece, Square};
        for (fen, color, piece, from, target, count) in [
            (
                "k3r3/8/8/8/8/8/4R3/4K3 w - - 0 1",
                Color::White,
                Piece::Rook,
                Square::E2,
                Square::E8,
                6,
            ),
            (
                "4k3/4r3/8/8/8/8/8/K3R3 b - - 0 1",
                Color::Black,
                Piece::Rook,
                Square::E7,
                Square::E1,
                6,
            ),
            (
                "k7/8/8/7b/8/8/4B3/3K4 w - - 0 1",
                Color::White,
                Piece::Bishop,
                Square::E2,
                Square::H5,
                3,
            ),
            (
                "3k4/4b3/8/8/7B/8/8/K7 b - - 0 1",
                Color::Black,
                Piece::Bishop,
                Square::E7,
                Square::H4,
                3,
            ),
            (
                "k7/8/8/8/8/5b2/4P3/3K4 w - - 0 1",
                Color::White,
                Piece::Pawn,
                Square::E2,
                Square::F3,
                1,
            ),
            (
                "3k4/4p3/5B2/8/8/8/8/K7 b - - 0 1",
                Color::Black,
                Piece::Pawn,
                Square::E7,
                Square::F6,
                1,
            ),
        ] {
            let board: Board = fen.parse().unwrap();
            let pins = super::pinned_pieces(&board, color);
            assert_eq!(pins, from.bitboard(), "{fen}");
            let raw = super::attacks_from(piece, from, color, board.occupied());
            let allowed = super::pin_restricted_attacks(raw, from, board.king(color), pins)
                & !board.colors(color);
            assert_eq!(allowed.len(), count, "{fen}");
            assert!(allowed.has(target), "the pinner remains capturable: {fen}");
            for to in allowed {
                assert!(
                    board.is_legal(Move {
                        from,
                        to,
                        promotion: None
                    }),
                    "{fen}: {from}{to}"
                );
            }
            for style in [false, true] {
                let summary = super::attack_summary_with_style(&board, style);
                let scan = summary.scans[color as usize];
                assert_eq!(scan.piece_mobility[piece as usize], count as i32, "{fen}");
                assert_eq!(scan.attacked, raw, "geometric control must survive: {fen}");
                assert_eq!(
                    scan.actionable,
                    raw & super::get_line_rays(board.king(color), from)
                );
            }
        }
    }

    #[test]
    fn pinned_pieces_preserve_geometric_king_safety() {
        use cozy_chess::{Piece, Square};
        for (fen, color, square, piece) in [
            (
                "k3r3/8/8/8/8/8/4N3/4K3 w - - 0 1",
                Color::White,
                Square::E2,
                Piece::Knight,
            ),
            (
                "4k3/4n3/8/8/8/8/8/K3R3 b - - 0 1",
                Color::Black,
                Square::E7,
                Piece::Knight,
            ),
            (
                "k3r3/8/8/8/8/8/4P3/4K3 w - - 0 1",
                Color::White,
                Square::E2,
                Piece::Pawn,
            ),
            (
                "4k3/4p3/8/8/8/8/8/K3R3 b - - 0 1",
                Color::Black,
                Square::E7,
                Piece::Pawn,
            ),
        ] {
            let board: Board = fen.parse().unwrap();
            let raw = super::attacks_from(piece, square, color, board.occupied());
            for style in [false, true] {
                let summary = super::attack_summary_with_style(&board, style);
                let scan = summary.scans[color as usize];
                assert_eq!(scan.attacked, raw);
                assert_eq!(scan.actionable, super::BitBoard::EMPTY);
                assert_eq!(scan.piece_mobility[piece as usize], 0);
            }
            assert_matches_reference(&board);
        }
    }

    #[test]
    fn multiple_blockers_and_wrong_slider_types_do_not_create_pins() {
        for fen in [
            "k3r3/8/8/8/8/4P3/4N3/4K3 w - - 0 1",
            "k3r3/8/8/8/8/4p3/4N3/4K3 w - - 0 1",
            "k3b3/8/8/8/8/8/4N3/4K3 w - - 0 1",
            "k7/8/8/7r/8/8/4N3/3K4 w - - 0 1",
        ] {
            let board: Board = fen.parse().unwrap();
            assert!(
                super::pinned_pieces(&board, Color::White).is_empty(),
                "{fen}"
            );
        }
    }

    #[test]
    fn evaluation_pins_match_move_generator_pins_over_a_playout() {
        let mut board = Board::default();
        for turn in 0..256 {
            assert_eq!(
                super::pinned_pieces(&board, board.side_to_move()),
                board.pinned() & board.colors(board.side_to_move()),
                "{board}"
            );
            let mut moves = Vec::new();
            board.generate_moves(|piece_moves| {
                moves.extend(piece_moves);
                false
            });
            if moves.is_empty() {
                board = Board::default();
            } else {
                board.play_unchecked(moves[(turn * 37 + 11) % moves.len()]);
            }
        }
    }

    #[cfg(feature = "tuning")]
    #[test]
    fn pinned_mobility_matches_the_tuning_vector() {
        use crate::engine::evaluation::tuning::{current_weights, tuning_features};
        let weights = current_weights();
        for fen in [
            "k3r3/8/8/8/5q2/8/4N3/4K3 w - - 0 1",
            "4k3/4n3/8/5Q2/8/8/8/K3R3 b - - 0 1",
            "k3r3/8/8/8/8/8/4R3/4K3 w - - 0 1",
            "4k3/4r3/8/8/8/8/8/K3R3 b - - 0 1",
            "k7/8/8/8/8/5b2/4P3/3K4 w - - 0 1",
            "3k4/4p3/5B2/8/8/8/8/K7 b - - 0 1",
        ] {
            let board: Board = fen.parse().unwrap();
            let expected = super::weights::score(&super::extract_with_style(&board, false));
            let vector = tuning_features(&board);
            let actual = vector
                .entries
                .iter()
                .fold((0, 0), |(mg, eg), &(index, count)| {
                    let weight = weights[index as usize];
                    (
                        mg + weight.0 * i32::from(count),
                        eg + weight.1 * i32::from(count),
                    )
                });
            assert_eq!(
                actual,
                (expected.middle_game(), expected.end_game()),
                "{fen}"
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

    /// The objective path's set-wise pawn scan must equal the styled path's
    /// per-pawn scan on everything the two share.
    #[test]
    fn the_objective_scan_matches_the_styled_scan_over_a_playout() {
        let mut board = Board::default();
        for turn in 0..256_usize {
            let styled = super::attack_summary_with_style(&board, true);
            let objective = super::attack_summary_with_style(&board, false);
            for color in [Color::White, Color::Black] {
                let mut expected = styled.scans[color as usize];
                expected.profile = Default::default();
                expected.attacker_mask = 0;
                expected.zone_defenders = 0;
                assert_eq!(
                    objective.scans[color as usize], expected,
                    "objective scan differs for {color:?} in {board}"
                );
            }
            assert_eq!(objective.blocked_passers, styled.blocked_passers);
            assert_eq!(objective.safe_checks, styled.safe_checks);
            assert_eq!(objective.threats, styled.threats);
            let mut moves = Vec::<Move>::new();
            board.generate_moves(|piece_moves| {
                moves.extend(piece_moves);
                false
            });
            if moves.is_empty() {
                board = Board::default();
                continue;
            }
            let chess_move = moves[(turn * 53 + 7) % moves.len()];
            board.play_unchecked(chess_move);
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

    #[test]
    fn safe_check_validation_recognizes_support_opened_by_the_checking_move() {
        use cozy_chess::{BitBoard, Piece, Square};
        let board: Board = "4k3/8/8/8/8/8/3R4/K2R4 w - - 0 1".parse().unwrap();
        let safe = super::safe_check_destinations(
            &board,
            Color::White,
            Piece::Rook,
            Square::D8.bitboard(),
            BitBoard::EMPTY,
            false,
        );
        assert!(safe.has(Square::D8));
        // The fitted prefilter does not add the newly uncovered checking square.
        assert_eq!(super::attack_summary(&board).safe_checks[0], [0, 0, 2, 0]);
    }

    #[test]
    fn safe_checks_reject_a_check_that_exposes_the_movers_king() {
        let board: Board = "4r2k/8/8/8/8/8/4R3/4K3 w - - 0 1".parse().unwrap();
        assert_eq!(super::attack_summary(&board).safe_checks[0], [0, 0, 1, 0]);
    }

    #[test]
    fn safe_checks_must_also_evade_an_existing_check() {
        let board: Board = "4k3/8/8/8/4N3/8/8/r6K w - - 0 1".parse().unwrap();
        assert_eq!(super::attack_summary(&board).safe_checks[0], [0; 4]);
    }

    fn legal_safe_check_destinations(board: &Board) -> [super::BitBoard; 4] {
        use cozy_chess::Piece;
        let mut result = [super::BitBoard::EMPTY; 4];
        let color = board.side_to_move();
        board.generate_moves(|moves| {
            let piece = board.piece_on(moves.from).unwrap();
            let slot = match piece {
                Piece::Knight => 0,
                Piece::Bishop => 1,
                Piece::Rook => 2,
                Piece::Queen => 3,
                Piece::Pawn | Piece::King => return false,
            };
            for chess_move in moves {
                let mut child = board.clone();
                child.play_unchecked(chess_move);
                if !child.checkers().has(chess_move.to) {
                    continue;
                }
                let mut capturable = false;
                child.generate_moves(|replies| {
                    if replies.to.has(chess_move.to) {
                        capturable = true;
                        true
                    } else {
                        false
                    }
                });
                if !capturable {
                    result[slot] |= chess_move.to.bitboard();
                }
            }
            false
        });
        assert_eq!(board.side_to_move(), color);
        result
    }

    fn filtered_safe_check_oracle(board: &Board) -> [i32; 4] {
        use super::{BitBoard, Piece};
        let color = board.side_to_move();
        let enemy = !color;
        let mut attacked = [BitBoard::EMPTY; 2];
        let mut twice = [BitBoard::EMPTY; 2];
        let mut type_attacks = [BitBoard::EMPTY; 4];
        for side in [color, enemy] {
            for piece in [
                Piece::Pawn,
                Piece::Knight,
                Piece::Bishop,
                Piece::Rook,
                Piece::Queen,
            ] {
                for square in board.colored_pieces(side, piece) {
                    let reach = super::attacks_from(piece, square, side, board.occupied());
                    twice[side as usize] |= attacked[side as usize] & reach;
                    attacked[side as usize] |= reach;
                    if side == color && piece != Piece::Pawn {
                        type_attacks[piece as usize - 1] |= reach;
                    }
                }
            }
        }
        let enemy_king = board.king(enemy);
        let bishop = super::get_bishop_moves(enemy_king, board.occupied());
        let rook = super::get_rook_moves(enemy_king, board.occupied());
        let checking = [
            super::get_knight_moves(enemy_king),
            bishop,
            rook,
            bishop | rook,
        ];
        let landing = !board.colors(color)
            & !attacked[enemy as usize]
            & (!super::get_king_moves(enemy_king) | twice[color as usize]);
        let legal = legal_safe_check_destinations(board);
        std::array::from_fn(|slot| {
            let candidates = type_attacks[slot] & checking[slot] & landing;
            if slot == 3 {
                candidates.len() as i32
            } else {
                (legal[slot] & candidates).len() as i32
            }
        })
    }

    fn mirror_safe_check_position(board: &Board) -> Board {
        let fen = board.to_string();
        let fields: Vec<_> = fen.split_whitespace().collect();
        assert_eq!(fields[2], "-");
        assert_eq!(fields[3], "-");
        let placement = fields[0]
            .split('/')
            .rev()
            .map(|rank| {
                rank.chars()
                    .map(|piece| {
                        if piece.is_ascii_uppercase() {
                            piece.to_ascii_lowercase()
                        } else {
                            piece.to_ascii_uppercase()
                        }
                    })
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("/");
        let side = if fields[1] == "w" { "b" } else { "w" };
        format!("{placement} {side} - - {} {}", fields[4], fields[5])
            .parse()
            .unwrap()
    }

    #[test]
    fn safe_check_occupancy_matches_legal_captures_and_color_mirrors() {
        for (fen, piece, destination, expected) in [
            ("4k3/8/8/8/8/8/8/3QK3 w - - 0 1", 3, "d8", false),
            ("4k3/8/8/8/8/8/3R4/K2R4 w - - 0 1", 2, "d8", true),
            ("4r2k/8/8/8/8/8/4R3/4K3 w - - 0 1", 2, "h2", false),
            ("4r2k/8/8/8/8/8/4R3/4K3 w - - 0 1", 2, "e8", true),
            ("4k3/8/8/8/4N3/8/8/r6K w - - 0 1", 0, "d6", false),
            ("4k3/8/8/8/4p3/8/8/3QK3 w - - 0 1", 3, "e2", false),
            ("7k/8/8/8/8/1Q6/b7/6RK w - - 0 1", 3, "g8", false),
            ("4k3/K7/1B6/2b5/8/8/8/3Q4 w - - 0 1", 3, "d8", true),
            ("4k3/4p3/8/8/2N5/8/8/K3R3 w - - 0 1", 0, "d6", true),
        ] {
            let board: Board = fen.parse().unwrap();
            let oracle = legal_safe_check_destinations(&board);
            assert_eq!(
                oracle[piece].has(destination.parse().unwrap()),
                expected,
                "{fen}"
            );
            let mirror = mirror_safe_check_position(&board);
            for position in [&board, &mirror] {
                let expected = filtered_safe_check_oracle(position);
                for style in [false, true] {
                    let actual = super::attack_summary_with_style(position, style);
                    assert_eq!(
                        actual.safe_checks[position.side_to_move() as usize],
                        expected,
                        "{position}"
                    );
                }
            }
            let original = super::attack_summary(&board).safe_checks;
            let flipped = super::attack_summary(&mirror).safe_checks;
            assert_eq!(original, [flipped[1], flipped[0]], "{fen}");
        }
    }

    #[test]
    fn safe_check_counts_match_legal_move_generation_over_playouts() {
        for seed in [7, 23, 53] {
            let mut board = Board::default();
            for turn in 0..128 {
                let expected = filtered_safe_check_oracle(&board);
                let actual = super::attack_summary_with_style(&board, false);
                assert_eq!(
                    actual.safe_checks[board.side_to_move() as usize],
                    expected,
                    "{board}"
                );
                if let Some(opposite) = board.null_move() {
                    let expected = filtered_safe_check_oracle(&opposite);
                    assert_eq!(
                        actual.safe_checks[opposite.side_to_move() as usize],
                        expected,
                        "other side of {board}"
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

    #[test]
    fn pawn_races_account_for_tempo_and_initial_double_pushes() {
        for (fen, expected) in [
            ("8/8/8/P3k3/8/8/8/7K w - - 0 1", 1),
            ("8/8/8/P3k3/8/8/8/7K b - - 0 1", 0),
            ("8/6k1/8/8/8/8/P7/7K w - - 0 1", 1),
            ("8/6k1/8/8/8/8/P7/7K b - - 0 1", 0),
            ("7K/8/8/8/8/8/P7/1k6 b - - 0 1", 0),
            ("8/2kPK3/8/8/8/8/8/8 w - - 0 1", 0),
        ] {
            let board: Board = fen.parse().unwrap();
            let mirrored = mirror_safe_check_position(&board);
            for style in [false, true] {
                assert_eq!(
                    super::extract_with_style(&board, style).pawn_race,
                    expected,
                    "{fen}"
                );
                assert_eq!(
                    super::extract_with_style(&mirrored, style).pawn_race,
                    -expected,
                    "{mirrored}"
                );
            }
        }
    }

    #[test]
    fn pawn_race_tempo_does_not_leak_through_the_structure_cache() {
        let first: Board = "8/8/8/P3k3/8/8/8/7K w - - 0 1".parse().unwrap();
        let second: Board = "8/8/8/P3k3/8/8/8/7K b - - 0 1".parse().unwrap();
        assert_eq!(
            super::StructureKey::new(&first),
            super::StructureKey::new(&second)
        );
        for _ in 0..8 {
            assert_eq!(super::extract(&first).pawn_race, 1);
            assert_eq!(super::extract(&second).pawn_race, 0);
        }
    }

    #[test]
    fn pawn_races_require_a_clear_path_and_no_interrupting_threat() {
        for fen in [
            "4K3/8/4P3/8/8/8/8/k7 w - - 0 1",
            "5k2/8/7p/P7/6K1/8/8/8 w - - 0 1",
            "5k2/P7/8/7p/6K1/8/8/8 w - - 0 1",
            "8/8/8/P3k3/8/8/8/5N1K w - - 0 1",
            "8/8/8/4k3/8/8/8/7K w - - 0 1",
            "5k2/P7/8/8/8/8/2K3p1/8 w - - 0 1",
            "7k/8/8/P7/8/8/5p2/6K1 w - - 0 1",
        ] {
            let board: Board = fen.parse().unwrap();
            assert_eq!(super::pawn_race(&board), 0, "{fen}");
            assert_eq!(
                super::pawn_race(&mirror_safe_check_position(&board)),
                0,
                "mirror of {fen}"
            );
        }
        let ep: Board = "7k/8/8/P2pP3/8/8/8/7K w - d6 0 1".parse().unwrap();
        assert!(ep.en_passant().is_some());
        assert_eq!(super::pawn_race(&ep), 0);

        let blocked: Board = "7k/8/P7/P7/8/8/8/7K w - - 0 1".parse().unwrap();
        assert!(!super::clear_pawn_run(
            &blocked,
            super::Square::A5,
            Color::White,
            5
        ));
        let double_blocked: Board = "7k/8/8/8/P7/8/P7/7K w - - 0 1".parse().unwrap();
        assert!(!super::clear_pawn_run(
            &double_blocked,
            super::Square::A2,
            Color::White,
            9
        ));
    }

    #[test]
    fn pawn_races_reward_at_most_one_runner_without_changing_style() {
        let board: Board = "7k/8/8/PP6/8/8/8/7K w - - 0 1".parse().unwrap();
        let mut features = super::extract(&board);
        assert_eq!(features.pawn_race, 1);
        let score = super::weights::score(&features);
        let style = super::weights::attacking_style(&features);
        features.pawn_race = 0;
        assert_eq!(
            score,
            super::weights::score(&features) + super::ScorePair::new(0, 80)
        );
        assert_eq!(style, super::weights::attacking_style(&features));
    }

    fn pawn_run_survives_legal_king_replies(
        board: &Board,
        color: Color,
        memo: &mut std::collections::HashMap<(super::Square, super::Square, Color), bool>,
    ) -> bool {
        use cozy_chess::{Piece, Rank};
        let Some(pawn) = board.colored_pieces(color, Piece::Pawn).into_iter().next() else {
            return false;
        };
        let key = (pawn, board.king(!color), board.side_to_move());
        if let Some(&result) = memo.get(&key) {
            return result;
        }
        let result = if board.side_to_move() == color {
            let (forward, start, promotion) = if color == Color::White {
                (1, Rank::Second, Rank::Eighth)
            } else {
                (-1, Rank::Seventh, Rank::First)
            };
            let to = pawn
                .try_offset(
                    0,
                    if pawn.rank() == start {
                        2 * forward
                    } else {
                        forward
                    },
                )
                .unwrap();
            let chess_move = Move {
                from: pawn,
                to,
                promotion: (to.rank() == promotion).then_some(Piece::Queen),
            };
            if !board.is_legal(chess_move) {
                false
            } else {
                let mut child = board.clone();
                child.play_unchecked(chess_move);
                if chess_move.promotion.is_some() {
                    let mut captured = false;
                    child.generate_moves(|replies| {
                        captured |= replies.to.has(to);
                        captured
                    });
                    !captured
                } else {
                    pawn_run_survives_legal_king_replies(&child, color, memo)
                }
            }
        } else {
            let mut has_reply = false;
            let mut survives = true;
            board.generate_moves(|replies| {
                for reply in replies {
                    has_reply = true;
                    let mut child = board.clone();
                    child.play_unchecked(reply);
                    if !pawn_run_survives_legal_king_replies(&child, color, memo) {
                        survives = false;
                        return true;
                    }
                }
                false
            });
            has_reply && survives
        };
        memo.insert(key, result);
        result
    }

    #[test]
    fn credited_pawn_runs_survive_every_legal_king_reply() {
        for fen in [
            "8/8/8/P3k3/8/8/8/7K w - - 0 1",
            "8/6k1/8/8/8/8/P7/7K w - - 0 1",
            "8/8/8/P5k1/8/8/8/7K b - - 0 1",
        ] {
            let board: Board = fen.parse().unwrap();
            for position in [&board, &mirror_safe_check_position(&board)] {
                let advantage = super::pawn_race(position);
                assert_ne!(advantage, 0, "{position}");
                let color = if advantage > 0 {
                    Color::White
                } else {
                    Color::Black
                };
                assert!(
                    pawn_run_survives_legal_king_replies(position, color, &mut Default::default()),
                    "{position}"
                );
            }
        }
    }

    #[cfg(feature = "tuning")]
    #[test]
    fn safe_check_occupancy_matches_the_tuning_vector() {
        use crate::engine::evaluation::tuning::{current_weights, tuning_features};
        let weights = current_weights();
        for fen in [
            "4k3/8/8/8/8/8/3R4/K2R4 w - - 0 1",
            "4r2k/8/8/8/8/8/4R3/4K3 w - - 0 1",
            "4k3/8/8/8/4N3/8/8/r6K w - - 0 1",
            "7k/8/8/8/8/1Q6/b7/6RK w - - 0 1",
            "4k3/K7/1B6/2b5/8/8/8/3Q4 w - - 0 1",
            "4k3/4p3/8/8/2N5/8/8/K3R3 w - - 0 1",
        ] {
            let board: Board = fen.parse().unwrap();
            for position in [&board, &mirror_safe_check_position(&board)] {
                let expected = super::weights::score(&super::extract_with_style(position, false));
                let vector = tuning_features(position);
                let actual = vector
                    .entries
                    .iter()
                    .fold((0, 0), |(mg, eg), &(index, count)| {
                        let weight = weights[index as usize];
                        (
                            mg + weight.0 * i32::from(count),
                            eg + weight.1 * i32::from(count),
                        )
                    });
                assert_eq!(
                    actual,
                    (expected.middle_game(), expected.end_game()),
                    "{position}"
                );
            }
        }
    }
}
