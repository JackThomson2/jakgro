//! Linear feature vector for offline weight fitting.
//!
//! The objective evaluation is a dot product. [`super::weights::score`] combines
//! scalar and indexed features with one weight pair each, and adds a placement
//! term that is itself a sum of piece-square entries. Written out, the whole
//! thing is
//!
//! ```text
//! score_mg = Σ w_mg[i] · x[i]      score_eg = Σ w_eg[i] · x[i]
//! ```
//!
//! over one vector `x` that this module produces, with the placement term
//! expanded from a precomputed pair back into the per-piece, per-square counts it
//! was summed from. A fitter can then treat the evaluation as ordinary linear
//! regression, and the engine and the fitter share one extraction rather than
//! two implementations that must be kept in agreement.
//!
//! What is deliberately *not* here: the profile mobility adjustment, which is
//! scaled by a per-profile intensity and so is not a fixed part of the objective
//! score, and the attacking-style weights, which are the personality. Both stay
//! exactly as written. Fitting them to game results would tune the engine's
//! character out of it, because game results do not reward interesting chess.
//!
//! This module is behind the `tuning` feature and is not built into the engine.

use cozy_chess::{Board, Color, Piece, Square};

use super::{
    BISHOP_MOBILITY_ENTRIES, KNIGHT_MOBILITY_ENTRIES, QUEEN_MOBILITY_ENTRIES,
    ROOK_MOBILITY_ENTRIES, Score, features, placement, weights,
};

/// How a block of weights is written back into the engine's source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    /// One `const NAME: ScorePair`.
    Scalar,
    /// A `const NAME: [ScorePair; N]`.
    Array,
    /// A `static NAME: Table`, laid out as two eight-by-eight grids.
    Table,
}

/// One contiguous group of features, named for the constant it is written to.
///
/// The vector's layout was previously stated in three places that had to agree
/// by hand: the order [`super::weights::score`] combines its terms in, the
/// arity of [`super::weights::tuning_weights`], and a parallel array of names in
/// the fitter used to emit source. Adding a feature meant editing all three, and
/// nothing would have caught a mismatch except a wrong evaluation.
///
/// Declaring it once here makes a new feature group one entry plus its
/// extraction, and lets the fitter emit source by walking this table.
#[derive(Clone, Copy, Debug)]
pub struct FeatureBlock {
    pub name: &'static str,
    pub offset: usize,
    pub len: usize,
    pub kind: BlockKind,
}

/// Declares a scalar block, so the table below reads as a list of names.
const fn scalar(name: &'static str, offset: usize) -> FeatureBlock {
    FeatureBlock {
        name,
        offset,
        len: 1,
        kind: BlockKind::Scalar,
    }
}

/// Declares a piece-square table block.
const fn table(name: &'static str, offset: usize) -> FeatureBlock {
    FeatureBlock {
        name,
        offset,
        len: 64,
        kind: BlockKind::Table,
    }
}

/// Declares an indexed block written as a `[ScorePair; N]`.
const fn array(name: &'static str, offset: usize, len: usize) -> FeatureBlock {
    FeatureBlock {
        name,
        offset,
        len,
        kind: BlockKind::Array,
    }
}

/// Every block in the vector, in index order.
///
/// The scalars come first, in the order `weights::score` combines them, then the
/// six piece-square tables. New groups append after the tables so that existing
/// indices, and therefore every recorded corpus and fitted weight file, keep
/// their meaning.
pub const BLOCKS: &[FeatureBlock] = &[
    scalar("PAWN", 0),
    scalar("KNIGHT", 1),
    scalar("BISHOP", 2),
    scalar("ROOK", 3),
    scalar("QUEEN", 4),
    scalar("ACTIVITY", 5),
    scalar("TEMPO", 6),
    scalar("PAWN_KING_MOBILITY", 7),
    scalar("BISHOP_PAIR", 8),
    scalar("DOUBLED_PAWN", 9),
    scalar("ISOLATED_PAWN", 10),
    array("PASSED_PAWN_BY_RANK", 11, 6),
    array("PROTECTED_PASSED_PAWN_BY_RANK", 17, 6),
    scalar("KING_SHELTER", 23),
    scalar("OPEN_KING_FILE", 24),
    table("PAWN", PLACEMENT_OFFSET),
    table("KNIGHT", PLACEMENT_OFFSET + 64),
    table("BISHOP", PLACEMENT_OFFSET + 128),
    table("ROOK", PLACEMENT_OFFSET + 192),
    table("QUEEN", PLACEMENT_OFFSET + 256),
    table("KING", PLACEMENT_OFFSET + 320),
    array(
        "KNIGHT_MOBILITY",
        TRAILING_OFFSET + MOBILITY_CURVES[0].1,
        MOBILITY_CURVES[0].2,
    ),
    array(
        "BISHOP_MOBILITY",
        TRAILING_OFFSET + MOBILITY_CURVES[1].1,
        MOBILITY_CURVES[1].2,
    ),
    array(
        "ROOK_MOBILITY",
        TRAILING_OFFSET + MOBILITY_CURVES[2].1,
        MOBILITY_CURVES[2].2,
    ),
    array(
        "QUEEN_MOBILITY",
        TRAILING_OFFSET + MOBILITY_CURVES[3].1,
        MOBILITY_CURVES[3].2,
    ),
    scalar("ROOK_OPEN_FILE", TRAILING_SCALAR_OFFSET),
    scalar("ROOK_SEMI_OPEN_FILE", TRAILING_SCALAR_OFFSET + 1),
    scalar("ROOK_ON_SEVENTH", TRAILING_SCALAR_OFFSET + 2),
    scalar("KNIGHT_OUTPOST", TRAILING_SCALAR_OFFSET + 3),
    scalar("BISHOP_OUTPOST", TRAILING_SCALAR_OFFSET + 4),
    scalar("BACKWARD_PAWN", TRAILING_SCALAR_OFFSET + 5),
    array("CONNECTED_PAWN_BY_RANK", CONNECTED_OFFSET, 6),
    array("BLOCKED_PASSER_BY_RANK", BLOCKED_PASSER_OFFSET, 6),
    array("PASSER_OWN_KING_DISTANCE", OWN_KING_DISTANCE_OFFSET, 8),
    array("PASSER_ENEMY_KING_DISTANCE", ENEMY_KING_DISTANCE_OFFSET, 8),
];

/// Scalar features before the tables, in the order [`super::weights::score`]
/// combines them.
pub const SCALAR_FEATURES: usize = 25;
/// Piece-square entries: six pieces over sixty-four squares.
pub const PLACEMENT_FEATURES: usize = 6 * 64;
/// Entries across the four mobility curves.
const MOBILITY_CURVE_ENTRIES: usize = KNIGHT_MOBILITY_ENTRIES
    + BISHOP_MOBILITY_ENTRIES
    + ROOK_MOBILITY_ENTRIES
    + QUEEN_MOBILITY_ENTRIES;
/// Scalar features after the mobility curves.
pub const TRAILING_SCALARS: usize = 6;
/// Features in the groups added after the tables.
pub const TRAILING_FEATURES: usize = MOBILITY_CURVE_ENTRIES + TRAILING_SCALARS + 6 + 6 + 8 + 8;
/// Length of the feature vector.
pub const FEATURE_COUNT: usize = SCALAR_FEATURES + PLACEMENT_FEATURES + TRAILING_FEATURES;
/// Index of the first piece-square feature.
pub const PLACEMENT_OFFSET: usize = SCALAR_FEATURES;
/// Index of the first feature after the tables.
pub const TRAILING_OFFSET: usize = PLACEMENT_OFFSET + PLACEMENT_FEATURES;
/// Index of the first scalar after the mobility curves.
const TRAILING_SCALAR_OFFSET: usize = TRAILING_OFFSET + MOBILITY_CURVE_ENTRIES;
/// Index of the connected-pawn block, after the trailing scalars.
const CONNECTED_OFFSET: usize = TRAILING_SCALAR_OFFSET + TRAILING_SCALARS;
/// Indices of the passer refinement blocks, in order.
const BLOCKED_PASSER_OFFSET: usize = CONNECTED_OFFSET + 6;
const OWN_KING_DISTANCE_OFFSET: usize = BLOCKED_PASSER_OFFSET + 6;
const ENEMY_KING_DISTANCE_OFFSET: usize = OWN_KING_DISTANCE_OFFSET + 8;
/// The mobility curves as piece, offset within the trailing region and length.
const MOBILITY_CURVES: [(Piece, usize, usize); 4] = [
    (Piece::Knight, 0, KNIGHT_MOBILITY_ENTRIES),
    (
        Piece::Bishop,
        KNIGHT_MOBILITY_ENTRIES,
        BISHOP_MOBILITY_ENTRIES,
    ),
    (
        Piece::Rook,
        KNIGHT_MOBILITY_ENTRIES + BISHOP_MOBILITY_ENTRIES,
        ROOK_MOBILITY_ENTRIES,
    ),
    (
        Piece::Queen,
        KNIGHT_MOBILITY_ENTRIES + BISHOP_MOBILITY_ENTRIES + ROOK_MOBILITY_ENTRIES,
        QUEEN_MOBILITY_ENTRIES,
    ),
];
/// Largest phase value, at which the middlegame weight applies alone.
pub const MAX_PHASE: i32 = 24;

/// One position's features, as the non-zero entries of a sparse vector.
///
/// Most of the vector is zero: at most thirty-two of the three hundred and
/// eighty-four placement features can be set, and several scalars are usually
/// zero too. A fitter walks millions of these, so the sparse form is what keeps
/// a pass over the corpus bounded by the pieces on the board.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TuningPosition {
    /// Feature index and its White-positive count.
    pub entries: Vec<(u16, i16)>,
    /// Game phase, from zero in a bare ending to [`MAX_PHASE`].
    pub phase: i32,
}

impl TuningPosition {
    /// Returns the blended score this vector produces under the given weights.
    ///
    /// This is the model the fitter optimizes, written once so the fit and the
    /// verification cannot drift apart.
    #[must_use]
    pub fn score(&self, weights: &[(Score, Score)]) -> Score {
        let mut middle_game = 0;
        let mut end_game = 0;
        for &(index, count) in &self.entries {
            let (weight_mg, weight_eg) = weights[index as usize];
            middle_game += weight_mg * Score::from(count);
            end_game += weight_eg * Score::from(count);
        }
        (middle_game * self.phase + end_game * (MAX_PHASE - self.phase)) / MAX_PHASE
    }
}

/// Extracts the linear feature vector for a position, from White's perspective.
#[must_use]
pub fn tuning_features(board: &Board) -> TuningPosition {
    let extracted = features::extract_with_style(board, false);
    let scalars = [
        extracted.pawns,
        extracted.knights,
        extracted.bishops,
        extracted.rooks,
        extracted.queens,
        extracted.activity,
        extracted.tempo,
        // The shared weight now covers only the two piece types without a
        // curve; the curves follow the tables.
        extracted.pawn_mobility + extracted.king_mobility,
        extracted.bishop_pair,
        extracted.doubled_pawns,
        extracted.isolated_pawns,
        extracted.passed_by_rank[0],
        extracted.passed_by_rank[1],
        extracted.passed_by_rank[2],
        extracted.passed_by_rank[3],
        extracted.passed_by_rank[4],
        extracted.passed_by_rank[5],
        extracted.protected_passer_by_rank[0],
        extracted.protected_passer_by_rank[1],
        extracted.protected_passer_by_rank[2],
        extracted.protected_passer_by_rank[3],
        extracted.protected_passer_by_rank[4],
        extracted.protected_passer_by_rank[5],
        extracted.king_shelter,
        extracted.open_king_files,
    ];

    let mut entries = Vec::with_capacity(SCALAR_FEATURES + 32);
    for (index, value) in scalars.into_iter().enumerate() {
        if value != 0 {
            entries.push((index as u16, value as i16));
        }
    }

    // Placement is accumulated per piece and square rather than read back from
    // the fused pair, which is what turns one blended number into the hundreds
    // of parameters that produced it.
    let mut placement_counts = [0_i16; PLACEMENT_FEATURES];
    for color in [Color::White, Color::Black] {
        let sign = if color == Color::White { 1 } else { -1 };
        for piece in Piece::ALL {
            for square in board.colored_pieces(color, piece) {
                placement_counts[placement_feature(piece, square, color)] += sign;
            }
        }
    }
    for (offset, count) in placement_counts.into_iter().enumerate() {
        if count != 0 {
            entries.push(((PLACEMENT_OFFSET + offset) as u16, count));
        }
    }

    // The mobility curves are expanded the same way: the engine accumulates a
    // weighted pair, and the fitter needs the count of pieces at each move
    // count that produced it.
    let mut curve_counts = [0_i16; TRAILING_FEATURES];
    for color in [Color::White, Color::Black] {
        let sign = if color == Color::White { 1 } else { -1 };
        for (piece, start, _) in MOBILITY_CURVES {
            debug_assert_eq!(weights::mobility_curve_offset(piece), Some(start));
            for square in board.colored_pieces(color, piece) {
                curve_counts[start + features::mobility_count(board, piece, square, color)] += sign;
            }
        }
    }
    for (offset, count) in curve_counts.into_iter().enumerate() {
        if count != 0 {
            entries.push(((TRAILING_OFFSET + offset) as u16, count));
        }
    }

    // Scalars after the curves, in the order `weights::trailing_scalars` lists
    // their weights.
    let trailing_scalars: [Score; TRAILING_SCALARS] = [
        extracted.rook_open_files,
        extracted.rook_semi_open_files,
        extracted.rooks_on_seventh,
        extracted.knight_outposts,
        extracted.bishop_outposts,
        extracted.backward_pawns,
    ];
    for (index, value) in trailing_scalars.into_iter().enumerate() {
        if value != 0 {
            entries.push(((TRAILING_SCALAR_OFFSET + index) as u16, value as i16));
        }
    }
    for (index, value) in extracted.connected_by_rank.into_iter().enumerate() {
        if value != 0 {
            entries.push(((CONNECTED_OFFSET + index) as u16, value as i16));
        }
    }
    for (index, value) in extracted.blocked_passer_by_rank.into_iter().enumerate() {
        if value != 0 {
            entries.push(((BLOCKED_PASSER_OFFSET + index) as u16, value as i16));
        }
    }
    for (index, value) in extracted.passer_own_king_distance.into_iter().enumerate() {
        if value != 0 {
            entries.push(((OWN_KING_DISTANCE_OFFSET + index) as u16, value as i16));
        }
    }
    for (index, value) in extracted.passer_enemy_king_distance.into_iter().enumerate() {
        if value != 0 {
            entries.push(((ENEMY_KING_DISTANCE_OFFSET + index) as u16, value as i16));
        }
    }

    TuningPosition {
        entries,
        phase: features::phase(board),
    }
}

/// Returns the weight vector the engine currently ships, in feature order.
#[must_use]
pub fn current_weights() -> Vec<(Score, Score)> {
    let mut weights: Vec<(Score, Score)> = weights::tuning_weights()
        .into_iter()
        .map(|pair| (pair.middle_game(), pair.end_game()))
        .collect();
    weights.extend(Piece::ALL.into_iter().flat_map(|piece| {
        (0..64).map(move |index| {
            let entry = placement::table_entry(piece, index);
            (entry.middle_game(), entry.end_game())
        })
    }));
    weights.extend(
        weights::trailing_tuning_weights()
            .into_iter()
            .map(|pair| (pair.middle_game(), pair.end_game())),
    );
    debug_assert_eq!(weights.len(), FEATURE_COUNT);
    weights
}

/// Maps a piece on a square to its placement feature index.
///
/// The index is the table row the engine reads, so a fitted weight can be
/// written straight back into the published table without a second mapping.
fn placement_feature(piece: Piece, square: Square, color: Color) -> usize {
    piece as usize * 64 + placement::table_index_for_tuning(square, color)
}

/// Returns a weight vector as the two source tables the engine reads.
///
/// Piece-square tables and material values are jointly under-determined: adding
/// a constant to every entry of one table and subtracting it from that piece's
/// material weight leaves every score unchanged. A fit therefore lands anywhere
/// along that ridge. Re-centring each table on zero and folding the mean it
/// carried into the material weight picks the one point on the ridge where the
/// tables say only where a piece belongs and the material weights say only what
/// it is worth, which is what both are documented to mean.
///
/// The king has no material weight, so the constant its table carries is not
/// merely arbitrary, it is unobservable: both sides always have exactly one, so
/// it cancels. It is centred and discarded.
#[must_use]
pub fn normalized(weights: &[(Score, Score)]) -> Vec<(Score, Score)> {
    let mut normalized = weights.to_vec();
    for piece in Piece::ALL {
        let start = PLACEMENT_OFFSET + piece as usize * 64;
        let squares = &mut normalized[start..start + 64];
        let occupiable: Vec<usize> = (0..64)
            .filter(|&index| piece != Piece::Pawn || (8..56).contains(&index))
            .collect();
        let count = occupiable.len() as Score;
        let mean_mg = occupiable.iter().map(|&i| squares[i].0).sum::<Score>() / count;
        let mean_eg = occupiable.iter().map(|&i| squares[i].1).sum::<Score>() / count;
        for &index in &occupiable {
            squares[index].0 -= mean_mg;
            squares[index].1 -= mean_eg;
        }
        // A pawn can never stand on the first or last rank, so those rows are
        // structurally zero rather than merely unvisited.
        if piece == Piece::Pawn {
            for index in (0..8).chain(56..64) {
                squares[index] = (0, 0);
            }
        }
        if let Some(material) = material_feature(piece) {
            normalized[material].0 += mean_mg;
            normalized[material].1 += mean_eg;
        }
    }
    normalized
}

/// The middlegame pawn weight every fit is rescaled to land on.
///
/// This is the scale the search's margins were last measured against, not a
/// round number chosen for looking like one. See [`anchored`].
pub const ANCHOR_MIDDLEGAME_PAWN: Score = 94;

/// Rescales a weight vector so the middlegame pawn lands on the anchor.
///
/// A logistic fit determines the evaluation only up to a positive scale: double
/// every weight and the same positions still rank the same way, and the loss is
/// recovered exactly by halving `K`. Nothing in the fit pins where along that
/// ray the answer lands, so each refit lands somewhere new.
///
/// Inside the objective evaluation that freedom is harmless, because a uniform
/// positive scale cannot reorder two scores. It is not harmless outside it,
/// because several numbers the search compares against an evaluation are fixed
/// centipawn constants that were tuned against whatever scale was current when
/// they were measured:
///
/// - the reverse-futility and quiet-futility margins, and the aspiration radius;
/// - the quiescence exchange threshold;
/// - [`super::piece_value`], a separate hardcoded table driving the swap list;
/// - and [`super::EvaluationConfig::style_middle_game_cap`], which bounds how
///   far the personality may move a score.
///
/// A refit that shrinks the evaluation by a tenth therefore widens every one of
/// those margins by a tenth and loosens the personality's cap, silently and
/// everywhere, having changed no line of search code. The third series' refit
/// moved the middlegame pawn from a round hundred to 94 and nothing recorded it.
///
/// Anchoring removes the freedom rather than compensating for it: the scale is
/// held where the margins were calibrated, so a later fit is free to change what
/// the evaluation *believes* without changing what the search's constants
/// *mean*. Moving the anchor is then a deliberate, separately measurable change
/// rather than a side effect of refitting.
#[must_use]
pub fn anchored(weights: &[(Score, Score)]) -> Vec<(Score, Score)> {
    let pawn = weights[material_feature(Piece::Pawn).expect("the pawn has a material weight")].0;
    if pawn <= 0 {
        // A fit that made a pawn worthless has failed in a way rescaling cannot
        // repair, and dividing by it would only hide that.
        return weights.to_vec();
    }
    weights
        .iter()
        .map(|&(middle_game, end_game)| (rescale(middle_game, pawn), rescale(end_game, pawn)))
        .collect()
}

/// Scales one weight by the anchor ratio, rounding half away from zero.
fn rescale(weight: Score, pawn: Score) -> Score {
    let scaled = f64::from(weight) * f64::from(ANCHOR_MIDDLEGAME_PAWN) / f64::from(pawn);
    scaled.round() as Score
}

/// Returns the scalar feature holding a piece's material weight, if it has one.
const fn material_feature(piece: Piece) -> Option<usize> {
    match piece {
        Piece::Pawn => Some(0),
        Piece::Knight => Some(1),
        Piece::Bishop => Some(2),
        Piece::Rook => Some(3),
        Piece::Queen => Some(4),
        Piece::King => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ANCHOR_MIDDLEGAME_PAWN, BLOCKS, BlockKind, FEATURE_COUNT, PLACEMENT_FEATURES,
        PLACEMENT_OFFSET, Piece, SCALAR_FEATURES, Score, TRAILING_FEATURES, TRAILING_OFFSET,
        anchored, current_weights, normalized, tuning_features,
    };
    use crate::engine::Position;
    use crate::engine::evaluation::{EvaluationConfig, MIN_AGGRESSION, weights};

    const POSITIONS: [&str; 6] = [
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
        "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        "r1bq1rk1/ppp2ppp/2n2n2/2b1p3/2B1P3/2NP1N2/PPP2PPP/R1BQ1RK1 w - - 0 8",
        "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
        "6k1/5ppp/8/7Q/2B5/8/5PPP/6K1 w - - 0 1",
        "4k3/8/8/8/8/8/4q3/4R1K1 b - - 0 1",
    ];

    /// The vector and the engine must agree exactly, or a fit optimizes a model
    /// the engine does not use.
    #[test]
    fn the_feature_vector_reproduces_the_objective_score() {
        let weights_now = current_weights();
        assert_eq!(weights_now.len(), FEATURE_COUNT);

        for fen in POSITIONS {
            let position = Position::from_fen(fen).unwrap();
            let extracted = super::features::extract_with_style(position.board(), false);
            let expected = weights::score(extracted);
            let vector = tuning_features(position.board());

            let mut middle_game = 0;
            let mut end_game = 0;
            for &(index, count) in &vector.entries {
                let (weight_mg, weight_eg) = weights_now[index as usize];
                middle_game += weight_mg * i32::from(count);
                end_game += weight_eg * i32::from(count);
            }

            assert_eq!(
                (middle_game, end_game),
                (expected.middle_game(), expected.end_game()),
                "feature vector disagreed on {fen}",
            );
        }
    }

    /// The shipped weights are already centred, so re-centring is a no-op.
    ///
    /// This was previously asserted the other way round, as a guard that the
    /// published tables still had a mean to fold out. That guard described the
    /// PeSTO tables the engine shipped before the evaluation refit; the refit
    /// adopted the output of `normalized`, which made the guard false and left
    /// the test failing on every checkout. It only ever ran under the `tuning`
    /// feature, which the default `cargo test` does not enable, so nothing said
    /// so.
    ///
    /// Centredness is the more useful property to pin anyway: it is what makes
    /// each material weight say only what a piece is worth and each table say
    /// only where it belongs, and a later fit that quietly reintroduced a mean
    /// would be a real regression.
    #[test]
    fn the_shipped_weights_are_already_centred() {
        let published = current_weights();

        assert_eq!(
            normalized(&published),
            published,
            "re-centring moved the shipped weights, so a mean has crept back in",
        );
    }

    /// The block table must describe exactly the vector everything else uses.
    ///
    /// This is the check that lets the layout be stated once. A block added
    /// without extraction, or extraction added without a block, shows up here
    /// as a gap, an overlap, or a length that does not reach `FEATURE_COUNT`.
    #[test]
    fn the_block_table_tiles_the_feature_vector() {
        let mut next = 0;
        for block in BLOCKS {
            assert_eq!(
                block.offset, next,
                "block {} starts at {} but the previous block ended at {next}",
                block.name, block.offset,
            );
            assert!(block.len > 0, "block {} is empty", block.name);
            next += block.len;
        }

        assert_eq!(
            next, FEATURE_COUNT,
            "the blocks cover {next} features but the vector holds {FEATURE_COUNT}",
        );
        assert_eq!(current_weights().len(), FEATURE_COUNT);
        // `SCALAR_FEATURES` counts slots, not blocks: one array block of six
        // contributes six. The region before the tables is what `weights.rs`
        // reads first and `PLACEMENT_OFFSET` ends; groups added later follow
        // the tables so the leading indices keep their meaning.
        assert_eq!(
            BLOCKS
                .iter()
                .filter(|block| block.offset < PLACEMENT_OFFSET)
                .map(|block| block.len)
                .sum::<usize>(),
            SCALAR_FEATURES,
        );
        assert_eq!(
            BLOCKS
                .iter()
                .filter(|block| block.offset >= TRAILING_OFFSET)
                .map(|block| block.len)
                .sum::<usize>(),
            TRAILING_FEATURES,
        );
        assert!(
            BLOCKS
                .iter()
                .filter(|block| block.offset >= TRAILING_OFFSET)
                .all(|block| block.kind != BlockKind::Table),
        );
        assert_eq!(
            BLOCKS
                .iter()
                .filter(|block| block.kind == BlockKind::Table)
                .map(|block| block.len)
                .sum::<usize>(),
            PLACEMENT_FEATURES,
        );
    }

    /// The anchor is a no-op on the shipped weights, which define it.
    #[test]
    fn the_shipped_weights_are_already_anchored() {
        let published = current_weights();

        assert_eq!(
            published[0].0, ANCHOR_MIDDLEGAME_PAWN,
            "the anchor no longer describes the shipped middlegame pawn",
        );
        assert_eq!(
            anchored(&published),
            published,
            "anchoring moved the shipped weights",
        );
    }

    /// A fit landing on a different scale is pulled back onto the anchor.
    ///
    /// This is the case the anchor exists for: the logistic fit is invariant
    /// under a positive scale, so a refit can land anywhere along the ray, and
    /// the search's fixed centipawn margins would silently change meaning.
    #[test]
    fn a_rescaled_fit_is_pulled_back_onto_the_anchor() {
        let published = current_weights();
        let inflated: Vec<(Score, Score)> = published
            .iter()
            .map(|&(middle_game, end_game)| (middle_game * 3, end_game * 3))
            .collect();

        let restored = anchored(&inflated);

        assert_eq!(restored[0].0, ANCHOR_MIDDLEGAME_PAWN);
        assert_eq!(
            restored, published,
            "an exactly tripled fit should divide back onto the published scale",
        );
    }

    /// Anchoring cannot reorder two positions, which is why it is safe to apply.
    #[test]
    fn anchoring_preserves_the_ranking_of_every_position() {
        let published = current_weights();
        let inflated: Vec<(Score, Score)> = published
            .iter()
            .map(|&(middle_game, end_game)| (middle_game * 7 / 2, end_game * 7 / 2))
            .collect();
        let restored = anchored(&inflated);

        let mut scores = Vec::new();
        for fen in POSITIONS {
            let position = Position::from_fen(fen).unwrap();
            let vector = tuning_features(position.board());
            scores.push((vector.score(&inflated), vector.score(&restored)));
        }

        for (index, left) in scores.iter().enumerate() {
            for right in &scores[index + 1..] {
                assert_eq!(
                    left.0.cmp(&right.0),
                    left.1.cmp(&right.1),
                    "anchoring reordered two positions",
                );
            }
        }
    }

    /// Re-centring moves weight between the tables and the material values
    /// without moving any score.
    #[test]
    fn normalization_preserves_every_score() {
        // Offset every knight square by a constant to build a deliberately
        // uncentred set. The shipped weights are centred, so normalizing them
        // exercises none of the arithmetic this test exists to check; a constant
        // added to one piece's table is exactly the mean `normalized` should
        // fold back into that piece's material weight.
        const OFFSET: Score = 7;
        let mut before = current_weights();
        let knight = PLACEMENT_OFFSET + Piece::Knight as usize * 64;
        for entry in &mut before[knight..knight + 64] {
            entry.0 += OFFSET;
            entry.1 += OFFSET;
        }

        let after = normalized(&before);
        assert_ne!(before, after, "the offset tables were left uncentred");

        // Deliberately not asserting that this recovers the shipped weights.
        // The mean is an integer division, so folding 7 back out of a table
        // whose own mean already truncates leaves a residue of a centipawn per
        // square — knight material lands on 336 rather than 330. Score
        // preservation is exact anyway, and it is the property that matters:
        // subtracting m from each of a piece's squares and adding m to its
        // material cannot move a score whatever m is.
        for fen in POSITIONS {
            let position = Position::from_fen(fen).unwrap();
            let vector = tuning_features(position.board());
            assert_eq!(
                vector.score(&before),
                vector.score(&after),
                "normalization moved the score of {fen}",
            );
        }
    }

    /// The blended model must match what search actually computes at the profile
    /// the fit targets.
    #[test]
    fn the_blended_model_matches_the_objective_evaluation() {
        let weights_now = current_weights();
        let objective = EvaluationConfig::new(MIN_AGGRESSION);

        for fen in POSITIONS {
            let position = Position::from_fen(fen).unwrap();
            let board = position.board();
            let modelled = tuning_features(board).score(&weights_now);
            let engine = super::super::evaluate_with_trace_and_config(board, objective).blended;

            assert_eq!(modelled, engine, "model disagreed with evaluation on {fen}");
        }
    }
}
