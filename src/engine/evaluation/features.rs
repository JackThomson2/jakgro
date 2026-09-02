use std::cell::RefCell;

use cozy_chess::{
    BitBoard, Board, Color, File, Piece, Rank, Square, get_bishop_moves, get_king_moves,
    get_knight_moves, get_pawn_attacks, get_rook_moves,
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
    /// by rank, passers by each king's distance, shelter by pawn distance —
    /// are sixty entries that the engine would otherwise copy out of the
    /// cache and multiply by their weights at every node, almost all of them
    /// zero. Weighting them once, on the miss, makes a hit one pair. The
    /// counts themselves are recomputed by [`structure_counts`] for the
    /// fitter and the tests, which are the only readers that need them.
    indexed: ScorePair,
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

thread_local! {
    /// Per-thread structure cache.
    ///
    /// Evaluation is a pure function of the board, so the cache is an
    /// optimization with no observable effect and needs no sharing between
    /// threads. Keeping it thread-local also keeps the search deterministic:
    /// whatever the cache state, a hit is verified against the full key.
    static STRUCTURE_CACHE: RefCell<Box<[(StructureKey, StructureTerms)]>> = RefCell::new(
        vec![(StructureKey::default(), StructureTerms::default()); STRUCTURE_CACHE_SLOTS]
            .into_boxed_slice(),
    );
}

/// Returns pawn and king structure terms, computing them only on a miss.
///
/// The empty key cannot collide with a real position, because every legal
/// position has two kings and `Square::A1` is square zero only for one of them at
/// a time; a slot still holding the default is simply a miss and is recomputed.
fn structure_terms(board: &Board) -> StructureTerms {
    let key = StructureKey::new(board);
    let slot = key.slot();
    STRUCTURE_CACHE.with(|cache| {
        if let Ok(mut cache) = cache.try_borrow_mut() {
            let (stored_key, stored) = cache[slot];
            if stored_key == key {
                return stored;
            }
            let terms = compute_structure_terms(board);
            cache[slot] = (key, terms);
            return terms;
        }
        compute_structure_terms(board)
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
        }
        let (shelter, open_files) = king_safety(board, color);
        counts.shelter += sign * shelter;
        counts.open_files += sign * open_files;
        let (king_file, adjacent) = shelter_distances(board, color);
        for distance in 0..6 {
            counts.shelter_king_file_by_distance[distance] += sign * i32::from(king_file[distance]);
            counts.shelter_adjacent_file_by_distance[distance] +=
                sign * i32::from(adjacent[distance]);
        }
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

    for color in [Color::White, Color::Black] {
        let sign = if color == Color::White { 1 } else { -1 };
        features.pawns += sign * board.colored_pieces(color, Piece::Pawn).len() as i32;
        features.knights += sign * board.colored_pieces(color, Piece::Knight).len() as i32;
        features.bishops += sign * board.colored_pieces(color, Piece::Bishop).len() as i32;
        features.rooks += sign * board.colored_pieces(color, Piece::Rook).len() as i32;
        features.queens += sign * board.colored_pieces(color, Piece::Queen).len() as i32;

        let bishops = board.colored_pieces(color, Piece::Bishop).len();
        features.bishop_pair += sign * i32::from(bishops >= 2);
        let scan = &attacks.scans[color as usize];
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

    // Pawn and king structure depends only on pawn placement and king squares, so
    // it is accumulated for both colours at once through the cache.
    let structure = structure_terms(board);
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

#[inline(always)]
fn centrality(square: Square) -> i32 {
    let file = square.file() as i32;
    let rank = square.rank() as i32;
    let file_distance = (file - 3).abs().min((file - 4).abs());
    let rank_distance = (rank - 3).abs().min((rank - 4).abs());
    6 - file_distance - rank_distance
}

#[cfg(test)]
fn reference_mobility(board: &Board, color: Color) -> i32 {
    let occupied = board.occupied();
    let friendly = board.colors(color);
    let mut total = 0;

    for piece in [
        Piece::Pawn,
        Piece::Knight,
        Piece::Bishop,
        Piece::Rook,
        Piece::Queen,
        Piece::King,
    ] {
        for square in board.colored_pieces(color, piece) {
            total += (attacks_from(piece, square, color, occupied) & !friendly).len() as i32;
        }
    }

    total
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
    passer_spans: &'static [BitBoard; 64],
    challenges: &'static [BitBoard; 64],
    outpost_ranks: BitBoard,
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
        let moves = attacks.len() as i32;
        scan.mobility += moves;
        scan.piece_mobility[PIECE] += moves;
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

        for target in attacks & context.enemy_pieces {
            let Some(target_piece) = board.piece_on(target) else {
                continue;
            };
            if !is_king
                && target_piece != Piece::King
                && piece_value(piece) < piece_value(target_piece)
            {
                scan.profile.threats += 1 + (piece_value(target_piece) - piece_value(piece)) / 100;
            }
        }

        scan.profile.space += attacks
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
    if STYLE {
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
            passer_spans,
            challenges,
            outpost_ranks,
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
            for target in board.colors(enemy) {
                let Some(target_piece) = board.piece_on(target) else {
                    continue;
                };
                if target_piece == Piece::King {
                    continue;
                }
                let attackers = i32::from(attack_counts[index][target as usize]);
                if attackers >= 2 {
                    result.supported_threats +=
                        (attackers - 1) * (1 + piece_value(target_piece) / 300);
                }
            }
        }
    }

    // A check is safe when the square is attacked by no enemy piece, or only
    // by the enemy king while a second friendly piece covers it. The checking
    // squares are the attacks of each piece type from the enemy king's square,
    // so the count is one intersection per type rather than a walk over moves.
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
        let minor_attacks = own.type_attacks[0] | own.type_attacks[1];
        summary.threats[index][0] = (enemy_minors & own.pawn_attacks).len() as i32;
        summary.threats[index][1] = (enemy_pieces & own_reach & !enemy_defends).len() as i32;
        summary.threats[index][2] = (enemy_majors & (own.pawn_attacks | minor_attacks)).len()
            as i32
            + (enemy_queens & own.type_attacks[2]).len() as i32;

        let safe = !theirs.attacked & (!king_reach | own.attacked_twice);
        let diagonals = get_bishop_moves(enemy_king, occupied);
        let lines = get_rook_moves(enemy_king, occupied);
        let checks = [
            get_knight_moves(enemy_king),
            diagonals,
            lines,
            diagonals | lines,
        ];
        let landing = !board.colors(color) & safe;
        for ((count, attacks), check_squares) in summary.safe_checks[index]
            .iter_mut()
            .zip(&own.type_attacks)
            .zip(&checks)
        {
            *count = (*attacks & *check_squares & landing).len() as i32;
        }
    }

    summary
}

/// Returns how many squares a piece may move to, for the fitter's expansion.
#[cfg(feature = "tuning")]
pub(super) fn mobility_count(board: &Board, piece: Piece, square: Square, color: Color) -> usize {
    (attacks_from(piece, square, color, board.occupied()) & !board.colors(color)).len() as usize
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
    for file in File::ALL {
        let count = (pawns & file.bitboard()).len();
        result.doubled += count.saturating_sub(1) as i32;
        if count > 0 && (pawns & file.adjacent()).is_empty() {
            result.isolated += count as i32;
        }
    }

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
}
