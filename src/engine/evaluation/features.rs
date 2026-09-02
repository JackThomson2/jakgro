use std::cell::RefCell;

use cozy_chess::{
    BitBoard, Board, Color, File, Piece, Rank, Square, get_bishop_moves, get_king_moves,
    get_knight_moves, get_pawn_attacks, get_rook_moves,
};

use super::{AttackProfile, EvalFeatures, ScorePair, piece_value, placement, weights};

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
    /// Passed pawns counted per rank, from the owner's side of the board.
    ///
    /// A pawn can only stand on relative ranks one to six, so six counters
    /// cover every case. They are `i8` because a count is bounded by eight and
    /// this structure sits in a direct-mapped cache whose whole point is fitting
    /// in a core's private cache.
    passed_by_rank: [i8; 6],
    /// Passed pawns defended by a friendly pawn, counted the same way.
    protected_passer_by_rank: [i8; 6],
    /// Connected pawns, counted the same way.
    connected_by_rank: [i8; 6],
    backward: i32,
    /// Passers by the distance from their owner's king to the square in
    /// front of them, and by the distance from the enemy king to it.
    passer_own_king_distance: [i8; 8],
    passer_enemy_king_distance: [i8; 8],
    shelter: i32,
    open_files: i32,
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
    let mut terms = StructureTerms::default();
    for color in [Color::White, Color::Black] {
        let sign = if color == Color::White { 1 } else { -1 };
        let pawns = pawn_features(board, color);
        terms.doubled += sign * pawns.doubled;
        terms.isolated += sign * pawns.isolated;
        terms.backward += sign * pawns.backward;
        for rank in 0..6 {
            terms.passed_by_rank[rank] += sign as i8 * pawns.passed_by_rank[rank];
            terms.protected_passer_by_rank[rank] +=
                sign as i8 * pawns.protected_passer_by_rank[rank];
            terms.connected_by_rank[rank] += sign as i8 * pawns.connected_by_rank[rank];
        }
        let (shelter, open_files) = king_safety(board, color);
        terms.shelter += sign * shelter;
        terms.open_files += sign * open_files;
        let (own, enemy) = passer_king_distances(board, color);
        for distance in 0..8 {
            terms.passer_own_king_distance[distance] += sign as i8 * own[distance];
            terms.passer_enemy_king_distance[distance] += sign as i8 * enemy[distance];
        }
    }
    terms
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
    let white_attack = attacks.profiles[Color::White as usize];
    let black_attack = attacks.profiles[Color::Black as usize];
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
        features.activity += sign * attacks.activity[color as usize];
        features.placement = features.placement + attacks.placement[color as usize] * sign;
        features.mobility += sign * attacks.mobility[color as usize];
        features.pawn_mobility +=
            sign * attacks.piece_mobility[color as usize][piece_index(Piece::Pawn) as usize];
        features.knight_mobility +=
            sign * attacks.piece_mobility[color as usize][piece_index(Piece::Knight) as usize];
        features.bishop_mobility +=
            sign * attacks.piece_mobility[color as usize][piece_index(Piece::Bishop) as usize];
        features.rook_mobility +=
            sign * attacks.piece_mobility[color as usize][piece_index(Piece::Rook) as usize];
        features.queen_mobility +=
            sign * attacks.piece_mobility[color as usize][piece_index(Piece::Queen) as usize];
        features.king_mobility +=
            sign * attacks.piece_mobility[color as usize][piece_index(Piece::King) as usize];
        features.mobility_curves =
            features.mobility_curves + attacks.mobility_curves[color as usize] * sign;
        let [open, semi_open, seventh] = attacks.rook_files[color as usize];
        features.rook_open_files += sign * open;
        features.rook_semi_open_files += sign * semi_open;
        features.rooks_on_seventh += sign * seventh;
        let [knights, bishops] = attacks.outposts[color as usize];
        features.knight_outposts += sign * knights;
        features.bishop_outposts += sign * bishops;
        for rank in 0..6 {
            features.blocked_passer_by_rank[rank] +=
                sign * attacks.blocked_passers[color as usize][rank];
        }
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
    for rank in 0..6 {
        features.passed_by_rank[rank] = i32::from(structure.passed_by_rank[rank]);
        features.protected_passer_by_rank[rank] =
            i32::from(structure.protected_passer_by_rank[rank]);
        features.connected_by_rank[rank] = i32::from(structure.connected_by_rank[rank]);
    }
    for distance in 0..8 {
        features.passer_own_king_distance[distance] =
            i32::from(structure.passer_own_king_distance[distance]);
        features.passer_enemy_king_distance[distance] =
            i32::from(structure.passer_enemy_king_distance[distance]);
    }
    // The attacking style weights passers by how far they have come, and that
    // term is personality and must not move. Deriving it from the per-rank
    // counts reproduces the old scalar exactly — it was the same sum — rather
    // than storing a second copy that could drift from them.
    features.passed_pawns = (0..6)
        .map(|rank| (rank as i32 + 1) * features.passed_by_rank[rank])
        .sum();
    features.king_shelter = structure.shelter;
    features.open_king_files = structure.open_files;

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
struct AttackSummary {
    profiles: [AttackProfile; 2],
    mobility: [i32; 2],
    piece_mobility: [[i32; 6]; 2],
    /// Per-piece mobility curves, already weighted, per colour.
    mobility_curves: [ScorePair; 2],
    /// Rooks on open files, semi-open files and the seventh, per colour.
    rook_files: [[i32; 3]; 2],
    /// Knights and bishops on outposts, per colour.
    outposts: [[i32; 2]; 2],
    /// Passers with a piece of either colour on the square ahead, by rank.
    blocked_passers: [[i32; 6]; 2],
    activity: [i32; 2],
    placement: [ScorePair; 2],
}

#[cfg(test)]
fn attack_summary(board: &Board) -> AttackSummary {
    attack_summary_with_style(board, true)
}

fn attack_summary_with_style(board: &Board, style: bool) -> AttackSummary {
    let occupied = board.occupied();
    let king_zones = [
        get_king_moves(board.king(Color::White)) | board.colored_pieces(Color::White, Piece::King),
        get_king_moves(board.king(Color::Black)) | board.colored_pieces(Color::Black, Piece::King),
    ];
    let mut profiles = [AttackProfile::default(); 2];
    let mut mobility = [0_i32; 2];
    let mut piece_mobility = [[0_i32; 6]; 2];
    let mut mobility_curves = [ScorePair::default(); 2];
    let mut rook_files = [[0_i32; 3]; 2];
    let mut outposts = [[0_i32; 2]; 2];
    let mut blocked_passers = [[0_i32; 6]; 2];
    let all_pawns = board.pieces(Piece::Pawn);
    let mut activity = [0_i32; 2];
    let mut placement = [ScorePair::default(); 2];
    let mut attack_counts = [[0_u8; 64]; 2];
    let mut zone_defenders = [0_i32; 2];

    for color in [Color::White, Color::Black] {
        let index = color as usize;
        let enemy = !color;
        let enemy_king = board.king(enemy);
        let enemy_king_zone = king_zones[enemy as usize];
        let enemy_pieces = board.colors(enemy);
        let friendly_pieces = board.colors(color);
        let own_pawns = board.colored_pieces(color, Piece::Pawn);
        let enemy_pawns = board.colored_pieces(enemy, Piece::Pawn);
        let (seventh, eighth) = if color == Color::White {
            (Rank::Seventh, Rank::Eighth)
        } else {
            (Rank::Second, Rank::First)
        };
        let (passer_spans, forward) = if color == Color::White {
            (&WHITE_PASSER_SPANS, 1)
        } else {
            (&BLACK_PASSER_SPANS, -1)
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
        let pawn_held = {
            let mut held = BitBoard::EMPTY;
            for pawn in own_pawns {
                held |= get_pawn_attacks(pawn, color);
            }
            held & outpost_ranks
        };
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
            let curve = weights::mobility_curve_offset(piece);
            for square in board.colored_pieces(color, piece) {
                // Placement and activity are accumulated in the same pass rather
                // than in a second loop over every piece: the terms differ but
                // the iteration is identical.
                placement[index] = placement[index] + placement::placement(piece, square, color);
                activity[index] += match piece {
                    Piece::Knight | Piece::Bishop | Piece::Rook | Piece::Queen => {
                        centrality(square)
                    }
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
                let raw_attacks = attacks_from(piece, square, color, occupied);
                let attacks = raw_attacks & !friendly_pieces;
                mobility[index] += attacks.len() as i32;
                piece_mobility[index][piece_index(piece) as usize] += attacks.len() as i32;
                // Weighted here for the same reason placement is: the curve is
                // a table lookup per piece, and expanding it into one count per
                // move count is work only the fitter needs.
                if let Some(offset) = curve {
                    mobility_curves[index] = mobility_curves[index]
                        + weights::mobility_curve_at(offset + attacks.len() as usize);
                }
                // A blockaded passer is one with any piece on the square ahead.
                // The passer test is a function of the pawns, but the blocker
                // is a piece, which is why this is here and not in the cache.
                if piece == Piece::Pawn
                    && (enemy_pawns & passer_spans[square as usize]).is_empty()
                    && square
                        .try_offset(0, forward)
                        .is_some_and(|stop| occupied.has(stop))
                {
                    let advance = if color == Color::White {
                        square.rank() as usize
                    } else {
                        7 - square.rank() as usize
                    };
                    blocked_passers[index][advance - 1] += 1;
                }
                if matches!(piece, Piece::Knight | Piece::Bishop)
                    && pawn_held.has(square)
                    && (enemy_pawns & challenges[square as usize]).is_empty()
                {
                    outposts[index][usize::from(piece == Piece::Bishop)] += 1;
                }
                if piece == Piece::Rook {
                    let file = square.file().bitboard();
                    if (all_pawns & file).is_empty() {
                        rook_files[index][0] += 1;
                    } else if (own_pawns & file).is_empty() {
                        rook_files[index][1] += 1;
                    }
                    // A rook on the seventh earns its name against a king it
                    // confines or pawns it attacks along the rank, not for the
                    // square alone.
                    if square.rank() == seventh
                        && (enemy_king.rank() == eighth
                            || !(enemy_pawns & seventh.bitboard()).is_empty())
                    {
                        rook_files[index][2] += 1;
                    }
                }
                if !style {
                    continue;
                }
                if piece != Piece::King {
                    for target in raw_attacks {
                        attack_counts[index][target as usize] += 1;
                    }
                    zone_defenders[index] +=
                        i32::from(!(raw_attacks & king_zones[index]).is_empty());
                }

                let zone_hits = (attacks & enemy_king_zone).len() as i32;
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
                        result.threats +=
                            1 + (piece_value(target_piece) - piece_value(piece)) / 100;
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
        result.king_pressure += result.attackers * result.attackers * 2;
        if !style {
            profiles[index] = result;
            continue;
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
        profiles[index] = result;
    }

    for color in [Color::White, Color::Black] {
        if !style {
            break;
        }
        let index = color as usize;
        let enemy = !color;
        let result = &mut profiles[index];
        result.defender_shortage = (result.attackers - zone_defenders[enemy as usize]).max(0);
        for target in board.colors(enemy) {
            let Some(target_piece) = board.piece_on(target) else {
                continue;
            };
            if target_piece == Piece::King {
                continue;
            }
            let attackers = i32::from(attack_counts[index][target as usize]);
            if attackers >= 2 {
                result.supported_threats += (attackers - 1) * (1 + piece_value(target_piece) / 300);
            }
        }
    }

    AttackSummary {
        profiles,
        mobility,
        piece_mobility,
        mobility_curves,
        rook_files,
        outposts,
        blocked_passers,
        activity,
        placement,
    }
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
    let mut enemy_attacks = BitBoard::EMPTY;
    for enemy in enemy_pawns {
        enemy_attacks |= get_pawn_attacks(enemy, !color);
    }

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

#[cfg(test)]
mod tests {
    use cozy_chess::{Board, Color, Move};

    use super::{attack_summary, reference_attacking_features, reference_mobility};

    fn assert_matches_reference(board: &Board) {
        let cached = attack_summary(board);
        for color in [Color::White, Color::Black] {
            assert_eq!(
                cached.profiles[color as usize],
                reference_attacking_features(board, color),
                "attack features differ for {color:?} in {board}"
            );
            assert_eq!(
                cached.mobility[color as usize],
                reference_mobility(board, color),
                "mobility differs for {color:?} in {board}"
            );
            assert_eq!(
                cached.activity[color as usize],
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
        let white_passed = super::compute_structure_terms(&white_passer).passed_by_rank;
        let black_passed = super::compute_structure_terms(&black_passer).passed_by_rank;
        assert_eq!(white_passed, [0, 0, 0, 1, 0, 0]);
        assert_eq!(black_passed, [0, 0, 0, -1, 0, 0]);
    }

    /// A protected passer is counted only where a friendly pawn defends it.
    #[test]
    fn protected_passers_are_counted_separately_from_bare_ones() {
        // b5 and c6: the c-pawn is passed and defended by the b-pawn, and the
        // b-pawn is passed and defended by nothing.
        let board: Board = "4k3/8/2P5/1P6/8/8/8/4K3 w - - 0 1".parse().unwrap();
        let terms = super::compute_structure_terms(&board);

        assert_eq!(terms.passed_by_rank, [0, 0, 0, 1, 1, 0]);
        assert_eq!(terms.protected_passer_by_rank, [0, 0, 0, 0, 1, 0]);
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
            let expected: i32 = (0..6)
                .map(|rank| (rank as i32 + 1) * features.passed_by_rank[rank])
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
