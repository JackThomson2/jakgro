use std::error::Error;
use std::io::{self, Cursor, Read};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use cozy_chess::{Board, Color, Move, Piece, Square};

use super::kernels::Row;
use super::{
    ACTIVATION_MAX, Accumulator, AccumulatorStack, FEATURE_SET, FILE_BYTES, FORMAT_VERSION,
    HEADER_BYTES, HIDDEN_SIZE, INPUT_FEATURES, LoadError, MAX_SCORE, Network, OUTPUT_BUCKETS,
    OUTPUT_SCALE, OUTPUT_UNIT, OUTPUT_WEIGHT_LIMIT, SCORE_SCALE, active_features, output_bucket,
};

fn synthetic_network() -> &'static Network {
    static MODEL: OnceLock<Network> = OnceLock::new();
    MODEL.get_or_init(|| {
        let mut seed = 0x17bd_673a_u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let model = Network {
            hidden_bias: Row(std::array::from_fn(|_| (next() % 401) as i16 - 80)),
            input_weights: (0..INPUT_FEATURES)
                .map(|_| Row(std::array::from_fn(|_| (next() % 65) as i16 - 32)))
                .collect(),
            output_weights: std::array::from_fn(|_| {
                std::array::from_fn(|_| Row(std::array::from_fn(|_| (next() % 255) as i16 - 127)))
            }),
            output_bias: std::array::from_fn(|bucket| -16_333 + 977 * bucket as i32),
        };
        Network::from_bytes(&encode(&model)).unwrap()
    })
}

fn zero_network() -> Network {
    Network {
        hidden_bias: Row([0; HIDDEN_SIZE]),
        input_weights: vec![Row([0; HIDDEN_SIZE]); INPUT_FEATURES].into_boxed_slice(),
        output_weights: [[Row([0; HIDDEN_SIZE]); 2]; OUTPUT_BUCKETS],
        output_bias: [0; OUTPUT_BUCKETS],
    }
}

fn encode(model: &Network) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(FILE_BYTES);
    bytes.extend_from_slice(b"JAKNNUE\0");
    for value in [
        3_u32,
        1,
        INPUT_FEATURES as u32,
        HIDDEN_SIZE as u32,
        255,
        64,
        (FILE_BYTES - HEADER_BYTES) as u32,
        OUTPUT_BUCKETS as u32,
    ] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(&0_u64.to_le_bytes());
    let rows = std::iter::once(&model.hidden_bias)
        .chain(model.input_weights.iter())
        .chain(model.output_weights.iter().flatten());
    for &value in rows.flat_map(|row| row.iter()) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for &value in &model.output_bias {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    assert_eq!(bytes.len(), FILE_BYTES);
    update_checksum(&mut bytes);
    bytes
}

fn update_checksum(bytes: &mut [u8]) {
    let mut hash = 14_695_981_039_346_656_037_u64;
    for &byte in &bytes[48..] {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(1_099_511_628_211);
    }
    bytes[40..48].copy_from_slice(&hash.to_le_bytes());
}

fn reference_features(board: &Board, perspective: Color) -> Vec<usize> {
    let mirror = board.king(perspective).file() as usize >= 4;
    let orient = |square: Square| {
        let rank = square.rank() as usize;
        let rank = if perspective == Color::White {
            rank
        } else {
            7 - rank
        };
        let file = square.file() as usize;
        let file = if mirror { 7 - file } else { file };
        rank * 8 + file
    };
    let king = orient(board.king(perspective));
    let bucket = (king / 16) * 2 + (king % 8) / 2;
    let mut features = Vec::new();
    for square in Square::ALL {
        let Some(piece) = board.piece_on(square) else {
            continue;
        };
        let kind = match piece {
            Piece::Pawn => 0,
            Piece::Knight => 1,
            Piece::Bishop => 2,
            Piece::Rook => 3,
            Piece::Queen => 4,
            Piece::King => 5,
        };
        let enemy = usize::from(board.color_on(square).unwrap() != perspective);
        features.push(bucket * 768 + (kind + enemy * 6) * 64 + orient(square));
    }
    features.sort_unstable();
    features
}

/// The independent scalar model: exact sums, then the squared clipped
/// activations dotted with the output weights in 64-bit arithmetic, scaled by
/// 400 and divided by 64 * 255 * 255 with truncation toward zero.
fn reference(model: &Network, board: &Board) -> ([[i32; HIDDEN_SIZE]; 2], i32) {
    let sums = [Color::White, Color::Black].map(|perspective| {
        let indices = reference_features(board, perspective);
        std::array::from_fn(|unit| {
            let total = i64::from(model.hidden_bias[unit])
                + indices
                    .iter()
                    .map(|&index| i64::from(model.input_weights[index][unit]))
                    .sum::<i64>();
            i32::try_from(total).unwrap()
        })
    });
    let bucket = (board.occupied().len() as usize - 1) / 4;
    let mut numerator = i64::from(model.output_bias[bucket]);
    for (slot, side) in [board.side_to_move(), !board.side_to_move()]
        .into_iter()
        .enumerate()
    {
        for (&weight, &sum) in model.output_weights[bucket][slot]
            .iter()
            .zip(&sums[side as usize])
        {
            let activation = i64::from(sum.clamp(0, 255));
            numerator += i64::from(weight) * activation * activation;
        }
    }
    let score = (numerator * 400 / 4_161_600).clamp(-16_000, 16_000) as i32;
    (sums, score)
}

fn assert_matches(model: &Network, board: &Board, accumulator: &Accumulator<'_>) {
    let (sums, score) = reference(model, board);
    for perspective in [Color::White, Color::Black] {
        assert_eq!(
            accumulator.values(perspective),
            sums[perspective as usize],
            "{board}"
        );
        assert_eq!(
            active_features(board, perspective)
                .into_iter()
                .map(usize::from)
                .collect::<Vec<_>>(),
            reference_features(board, perspective),
            "{board}"
        );
    }
    assert_eq!(accumulator.evaluate(), score, "{board}");
    assert_eq!(model.evaluate(board), score, "{board}");
}

fn mirror(board: &Board) -> Board {
    let fen = board.to_string();
    let fields = fen.split_whitespace().collect::<Vec<_>>();
    assert_eq!(&fields[2..4], &["-", "-"]);
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
    let side = if board.side_to_move() == Color::White {
        "b"
    } else {
        "w"
    };
    format!("{placement} {side} - - 0 1").parse().unwrap()
}

#[test]
fn feature_and_file_contract_has_explicit_dimensions() {
    assert_eq!(FORMAT_VERSION, 3);
    assert_eq!(FEATURE_SET, 1);
    assert_eq!(OUTPUT_BUCKETS, 8);
    assert_eq!(INPUT_FEATURES, 6_144);
    assert_eq!(HIDDEN_SIZE, 512);
    assert_eq!(ACTIVATION_MAX, 255);
    assert_eq!(OUTPUT_SCALE, 64);
    assert_eq!(OUTPUT_WEIGHT_LIMIT, 127);
    assert_eq!(SCORE_SCALE, 400);
    assert_eq!(OUTPUT_UNIT, 4_161_600);
    // One centipawn of output bias is exactly this many numerator units.
    assert_eq!(OUTPUT_UNIT / SCORE_SCALE, 10_404);
    assert_eq!(OUTPUT_UNIT % SCORE_SCALE, 0);
    assert_eq!(HEADER_BYTES, 48);
    assert_eq!(
        FILE_BYTES,
        HEADER_BYTES
            + 2 * (HIDDEN_SIZE + INPUT_FEATURES * HIDDEN_SIZE + OUTPUT_BUCKETS * 2 * HIDDEN_SIZE)
            + 4 * OUTPUT_BUCKETS
    );
    assert_eq!(FILE_BYTES, 6_308_944);
}

#[test]
fn feature_indices_fix_orientation_planes_and_buckets() {
    let board: Board = "4k3/8/5n2/8/8/8/2P5/4K3 w - - 0 1".parse().unwrap();
    assert_eq!(
        active_features(&board, Color::White),
        [781, 1091, 1258, 1531]
    );
    assert_eq!(
        active_features(&board, Color::Black),
        [850, 1091, 1205, 1531]
    );
    // The horizontal reflection of a position has the same features.
    let reflected: Board = "3k4/8/2n5/8/8/8/5P2/3K4 w - - 0 1".parse().unwrap();
    assert_eq!(
        active_features(&reflected, Color::White),
        active_features(&board, Color::White)
    );
    assert_eq!(
        active_features(&reflected, Color::Black),
        active_features(&board, Color::Black)
    );
    for perspective in [Color::White, Color::Black] {
        for king in Square::ALL {
            let rank = if perspective == Color::White {
                king.rank() as usize
            } else {
                7 - king.rank() as usize
            };
            let file = king.file() as usize;
            let view = super::view(king, perspective);
            assert_eq!(view.mirror, file >= 4);
            assert_eq!(
                usize::from(view.bucket),
                (rank / 2) * 2 + file.min(7 - file) / 2
            );
        }
    }
    let starting = Board::default();
    let white = active_features(&starting, Color::White);
    assert_eq!(white.len(), 32);
    assert_eq!(white, active_features(&starting, Color::Black));
    assert!(white.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(
        white
            .iter()
            .all(|&index| usize::from(index) < INPUT_FEATURES)
    );
}

#[test]
fn binary_round_trip_preserves_all_tensors_and_signed_values() {
    let model = synthetic_network();
    let bytes = encode(model);
    let loaded = Network::read_from(Cursor::new(&bytes)).unwrap();
    assert_eq!(loaded.hidden_bias, model.hidden_bias);
    assert_eq!(loaded.input_weights, model.input_weights);
    assert_eq!(loaded.output_weights, model.output_weights);
    assert_eq!(loaded.output_bias, model.output_bias);
    assert_eq!(encode(&loaded), bytes);
    let board = Board::default();
    assert_matches(&loaded, &board, &loaded.accumulator(&board));
}

#[test]
fn full_inference_matches_independent_scalar_reference() {
    let model = synthetic_network();
    for fen in [
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
        "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        "4k3/8/5n2/8/8/8/2P5/4K3 w - - 0 1",
        "8/8/3k4/8/5K2/2P5/8/8 b - - 17 45",
        "QQ5k/8/8/8/8/8/8/K7 b - - 0 1",
    ] {
        let board: Board = fen.parse().unwrap();
        assert_matches(model, &board, &model.accumulator(&board));
    }
}

#[test]
fn color_swapping_and_rank_mirroring_preserve_relative_scores() {
    let model = synthetic_network();
    for fen in [
        "4k3/8/5n2/8/8/8/2P5/4K3 w - - 0 1",
        "8/8/3k4/8/5K2/2P5/8/8 b - - 0 1",
        "6k1/5ppp/8/7Q/2B5/8/5PPP/6K1 w - - 0 1",
    ] {
        let board: Board = fen.parse().unwrap();
        let flipped = mirror(&board);
        for side in [Color::White, Color::Black] {
            assert_eq!(
                active_features(&board, side),
                active_features(&flipped, !side)
            );
        }
        assert_eq!(model.evaluate(&board), model.evaluate(&flipped));
    }
}

#[test]
fn incremental_special_moves_and_restoration_match_refresh() {
    let model = synthetic_network();
    let fixtures = [
        ("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1", "e1h1"),
        ("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1", "e1a1"),
        ("r3k2r/8/8/8/8/8/8/R3K2R b KQkq - 0 1", "e8h8"),
        ("r3k2r/8/8/8/8/8/8/R3K2R b KQkq - 0 1", "e8a8"),
        ("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1", "e5d6"),
        ("4k3/8/8/8/3Pp3/8/8/4K3 b - d3 0 1", "e4d3"),
        ("4k3/8/8/3r4/3R4/8/8/4K3 w - - 0 1", "d4d5"),
        ("7k/8/8/8/8/8/8/4K3 w - - 0 1", "e1f1"),
        ("7k/8/8/8/8/8/8/4K3 w - - 0 1", "e1d1"),
    ];
    for (fen, text) in fixtures {
        let board: Board = fen.parse().unwrap();
        let chess_move: Move = text.parse().unwrap();
        assert!(board.is_legal(chess_move), "{fen}: {text}");
        let mut child = board.clone();
        child.play_unchecked(chess_move);
        let mut accumulator = model.accumulator(&board);
        for position in [&child, &board, &child, &child, &board] {
            accumulator.update(position);
            assert_matches(model, position, &accumulator);
        }
    }
}

#[test]
fn every_promotion_and_underpromotion_updates_both_perspectives() {
    let model = synthetic_network();
    for (fen, from, to) in [
        ("7k/P7/8/8/8/8/8/K7 w - - 0 1", Square::A7, Square::A8),
        ("1r5k/P7/8/8/8/8/8/K7 w - - 0 1", Square::A7, Square::B8),
        ("7k/8/8/8/8/8/p7/7K b - - 0 1", Square::A2, Square::A1),
        ("7k/8/8/8/8/8/p7/1R5K b - - 0 1", Square::A2, Square::B1),
    ] {
        let board: Board = fen.parse().unwrap();
        let mut accumulator = model.accumulator(&board);
        for piece in [Piece::Knight, Piece::Bishop, Piece::Rook, Piece::Queen] {
            let chess_move = Move {
                from,
                to,
                promotion: Some(piece),
            };
            assert!(board.is_legal(chess_move));
            let mut child = board.clone();
            child.play_unchecked(chess_move);
            accumulator.update(&child);
            assert_matches(model, &child, &accumulator);
            accumulator.update(&board);
            assert_matches(model, &board, &accumulator);
        }
    }
}

#[test]
fn null_moves_swap_output_order_without_changing_sums() {
    let mut bytes = encode(&zero_network());
    // White's c2 pawn drives one unit to 204, below the clip; its squared
    // activation times a weight of 3 is exactly twelve centipawns:
    // 3 * 204 * 204 * 400 / 4_161_600 = 12.
    let row = 781;
    let input = HEADER_BYTES + 2 * HIDDEN_SIZE + 2 * row * HIDDEN_SIZE;
    bytes[input..input + 2].copy_from_slice(&204_i16.to_le_bytes());
    let output = HEADER_BYTES + 2 * (HIDDEN_SIZE + INPUT_FEATURES * HIDDEN_SIZE);
    bytes[output..output + 2].copy_from_slice(&3_i16.to_le_bytes());
    update_checksum(&mut bytes);
    let model = Network::from_bytes(&bytes).unwrap();
    let board: Board = "4k3/8/5n2/8/8/8/2P5/4K3 w - - 0 1".parse().unwrap();
    let null = board.null_move().unwrap();
    let mut accumulator = model.accumulator(&board);
    let sums = accumulator.state.sums;
    assert_eq!(accumulator.values(Color::White)[0], 204);
    assert_eq!(accumulator.evaluate(), 12);
    accumulator.update(&null);
    assert_eq!(accumulator.state.sums, sums);
    assert_eq!(accumulator.evaluate(), 0);
    assert_matches(&model, &null, &accumulator);
}

#[test]
fn metadata_does_not_change_piece_only_features_or_scores() {
    let model = synthetic_network();
    for (first, second) in [
        (
            "r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1",
            "r3k2r/8/8/8/8/8/8/R3K2R w - - 90 40",
        ),
        (
            "4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1",
            "4k3/8/8/3pP3/8/8/8/4K3 w - - 77 50",
        ),
    ] {
        let first: Board = first.parse().unwrap();
        let second: Board = second.parse().unwrap();
        let mut accumulator = model.accumulator(&first);
        let sums = accumulator.state.sums;
        let score = accumulator.evaluate();
        accumulator.update(&second);
        assert_eq!(accumulator.state.sums, sums);
        assert_eq!(accumulator.evaluate(), score);
    }
}

#[test]
fn incremental_playouts_siblings_and_clones_match_independent_reference() {
    let model = synthetic_network();
    for seed in [7_usize, 19, 37, 53] {
        let mut board = Board::default();
        let mut accumulator = model.accumulator(&board);
        for turn in 0..192 {
            assert_matches(model, &board, &accumulator);
            if let Some(null) = board.null_move() {
                let mut probe = accumulator.clone();
                probe.update(&null);
                assert_matches(model, &null, &probe);
                assert_matches(model, &board, &accumulator);
            }
            let mut moves = Vec::new();
            board.generate_moves(|batch| {
                moves.extend(batch);
                false
            });
            if moves.is_empty() {
                board = Board::default();
                accumulator.update(&board);
                continue;
            }
            let chosen = moves[(turn * seed + 11) % moves.len()];
            let mut sibling = board.clone();
            sibling.play_unchecked(moves[(turn * seed + 17) % moves.len()]);
            accumulator.update(&sibling);
            assert_matches(model, &sibling, &accumulator);
            board.play_unchecked(chosen);
            accumulator.update(&board);
        }
    }
}

/// Walks a small tree the way a search does: a child is evaluated one ply
/// below its parent, siblings reuse a ply, and some nodes are never evaluated
/// so the states above and beside a node are stale.
fn walk(
    model: &Network,
    stack: &mut AccumulatorStack<'_>,
    board: &Board,
    ply: usize,
    depth: usize,
    seed: &mut u64,
) {
    *seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    if !(*seed >> 33).is_multiple_of(4) {
        let (sums, score) = reference(model, board);
        assert_eq!(stack.evaluate(board, ply), score, "{board}");
        for perspective in [Color::White, Color::Black] {
            assert_eq!(
                stack.values(ply, perspective),
                sums[perspective as usize],
                "{board}"
            );
        }
    }
    if depth == 0 {
        return;
    }
    let mut moves = Vec::new();
    board.generate_moves(|batch| {
        moves.extend(batch);
        false
    });
    for offset in 0..moves.len().min(4) {
        let chosen = moves[((*seed >> 20) as usize + offset * 7) % moves.len()];
        let mut child = board.clone();
        child.play_unchecked(chosen);
        walk(model, stack, &child, ply + 1, depth - 1, seed);
    }
}

#[test]
fn stack_matches_independent_reference_over_search_shaped_walks() {
    let model = synthetic_network();
    for (fen, depth) in [
        (
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            5,
        ),
        (
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            5,
        ),
        // Bare kings cross king buckets and the mirror line at almost every
        // move, so perspectives are rebuilt beside incremental ones.
        ("8/5pk1/6p1/3R4/1p3P2/1P4P1/r5KP/8 w - - 0 40", 6),
        ("8/2k5/8/3K4/8/8/1P4p1/8 b - - 0 1", 6),
        ("r3k2r/8/8/8/8/8/8/R3K2R b KQkq - 0 1", 5),
    ] {
        let board: Board = fen.parse().unwrap();
        let mut stack = AccumulatorStack::new(model, 8);
        let mut seed = 0x9e37_79b9_u64;
        for _ in 0..3 {
            walk(model, &mut stack, &board, 0, depth, &mut seed);
        }
    }
}

#[test]
fn stack_shares_its_deepest_state_and_serves_unrelated_boards() {
    let model = synthetic_network();
    let mut stack = AccumulatorStack::new(model, 2);
    let boards: Vec<Board> = [
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
        "7k/8/8/8/8/8/8/K7 w - - 0 1",
        "QQ5k/8/8/8/8/8/8/K7 b - - 0 1",
        "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        "7k/8/8/8/8/8/8/K7 b - - 0 1",
    ]
    .iter()
    .map(|fen| fen.parse().unwrap())
    .collect();
    for (turn, ply) in [0, 2, 1, 7, usize::MAX, 0, 2, 1, 1, 0]
        .into_iter()
        .enumerate()
    {
        let board = &boards[turn * 3 % boards.len()];
        assert_eq!(
            stack.evaluate(board, ply),
            reference(model, board).1,
            "{board}"
        );
    }
}

#[test]
fn activation_clipping_squaring_and_negative_division_are_explicit() {
    let mut model = zero_network();
    model.hidden_bias[..4].copy_from_slice(&[-5, 102, 255, 300]);
    let board = Board::default();
    let bucket = output_bucket(&board);
    assert_eq!(bucket, 7);
    // -5 clips to zero whatever its weight, 300 clips to 255 and cancels the
    // 255 unit, and 102^2 = 10_404 numerator units is exactly one centipawn
    // (a linear activation would truncate 102 * 400 / 4_161_600 to zero).
    model.output_weights[bucket][0][..4].copy_from_slice(&[127, 1, -1, 1]);
    assert_eq!(
        model.accumulator(&board).values(Color::White)[..4],
        [-5, 102, 255, 300]
    );
    assert_eq!(
        Network::from_bytes(&encode(&model))
            .unwrap()
            .evaluate(&board),
        1
    );
    // The same unit at 204 squares to four centipawns, not two.
    model.hidden_bias[1] = 204;
    assert_eq!(
        Network::from_bytes(&encode(&model))
            .unwrap()
            .evaluate(&board),
        4
    );
    model.output_weights = [[Row([0; HIDDEN_SIZE]); 2]; OUTPUT_BUCKETS];
    for (bias, expected) in [
        (-10_403, 0),
        (-10_404, -1),
        (-10_405, -1),
        (10_403, 0),
        (10_404, 1),
    ] {
        model.output_bias[bucket] = bias;
        assert_eq!(
            Network::from_bytes(&encode(&model))
                .unwrap()
                .evaluate(&board),
            expected
        );
    }
}

#[test]
fn extreme_weights_keep_unclipped_incremental_sums_and_bounded_output() {
    // The largest uniform weights whose 64-square bound still fits an i16.
    for (weight, bias) in [(511_i16, 63_i16), (-512, 0)] {
        let mut model = zero_network();
        model.hidden_bias = Row([bias; HIDDEN_SIZE]);
        model.input_weights.fill(Row([weight; HIDDEN_SIZE]));
        model.output_weights = [[Row([-OUTPUT_WEIGHT_LIMIT; HIDDEN_SIZE]); 2]; OUTPUT_BUCKETS];
        let model = Network::from_bytes(&encode(&model)).unwrap();
        let board = Board::default();
        let sparse: Board = "7k/8/8/8/8/8/8/K7 w - - 0 1".parse().unwrap();
        let mut accumulator = model.accumulator(&board);
        assert_eq!(
            accumulator.values(Color::White)[0],
            i32::from(bias) + 32 * i32::from(weight)
        );
        for position in [&sparse, &board, &sparse, &board] {
            accumulator.update(position);
            assert_matches(&model, position, &accumulator);
        }
    }
    let mut model = zero_network();
    for (bias, expected) in [
        (i32::MAX, MAX_SCORE),
        (-i32::MAX, -MAX_SCORE),
        (i32::MIN, -MAX_SCORE),
    ] {
        model.output_bias = [bias; OUTPUT_BUCKETS];
        assert_eq!(
            Network::from_bytes(&encode(&model))
                .unwrap()
                .evaluate(&Board::default()),
            expected
        );
    }
}

#[test]
fn unprovable_accumulator_bounds_are_rejected_at_the_exact_boundary() {
    // One unit of the last bucket; every square's extreme weight sits on a
    // different plane, so the bound has to take the extreme over planes.
    for sign in [1_i32, -1] {
        let mut model = zero_network();
        let limit = if sign == 1 { 32_767 } else { 32_768 };
        let bucket = (super::KING_BUCKETS - 1) * super::PIECE_PLANES * 64;
        for square in 0..64 {
            let plane = square % super::PIECE_PLANES;
            model.input_weights[bucket + plane * 64 + square][5] = (sign * 500) as i16;
            // The opposite sign never helps this side of the bound.
            model.input_weights[bucket + (plane + 1) % 12 * 64 + square][5] = (-sign * 9) as i16;
        }
        model.hidden_bias[5] = (sign * (limit - 64 * 500)) as i16;
        assert!(Network::from_bytes(&encode(&model)).is_ok());
        model.hidden_bias[5] += sign as i16;
        assert!(matches!(
            Network::from_bytes(&encode(&model)),
            Err(LoadError::AccumulatorOverflow)
        ));
        // A lone weight outside the bound is enough, whatever the other rows hold.
        let mut model = zero_network();
        model.input_weights[0][HIDDEN_SIZE - 1] = (sign * 32_767) as i16;
        model.hidden_bias[HIDDEN_SIZE - 1] = (sign * 2) as i16;
        assert!(matches!(
            Network::from_bytes(&encode(&model)),
            Err(LoadError::AccumulatorOverflow)
        ));
    }
}

#[test]
fn output_weights_beyond_the_limit_are_rejected_at_the_exact_boundary() {
    for sign in [1_i16, -1] {
        let mut model = zero_network();
        // A single weight in one row of one bucket decides it; the bias is
        // free to take any value.
        model.output_bias = [i32::MIN; OUTPUT_BUCKETS];
        model.output_bias[OUTPUT_BUCKETS - 1] = i32::MAX;
        model.output_weights[OUTPUT_BUCKETS - 1][1][HIDDEN_SIZE - 1] = sign * OUTPUT_WEIGHT_LIMIT;
        assert!(Network::from_bytes(&encode(&model)).is_ok());
        model.output_weights[OUTPUT_BUCKETS - 1][1][HIDDEN_SIZE - 1] =
            sign * (OUTPUT_WEIGHT_LIMIT + 1);
        assert!(matches!(
            Network::from_bytes(&encode(&model)),
            Err(LoadError::OutputOverflow)
        ));
    }
}

#[test]
fn malformed_header_fields_are_rejected_before_reading_the_payload() {
    let bytes = encode(synthetic_network());
    for offset in [0, 8, 12, 16, 20, 24, 28, 32, 36] {
        let mut changed = bytes.clone();
        changed[offset] ^= 0xff;
        assert!(matches!(
            Network::from_bytes(&changed),
            Err(LoadError::Header(_))
        ));
        let mut reader = Cursor::new(changed);
        assert!(matches!(
            Network::read_from(&mut reader),
            Err(LoadError::Header(_))
        ));
        assert_eq!(reader.position(), HEADER_BYTES as u64);
    }
}

#[test]
fn truncated_trailing_and_corrupted_models_are_rejected() {
    let bytes = encode(synthetic_network());
    for length in [0, 7, 47, 48, 511, FILE_BYTES - 1] {
        assert!(matches!(
            Network::from_bytes(&bytes[..length]),
            Err(LoadError::Length { .. })
        ));
        assert!(
            matches!(Network::read_from(&bytes[..length]), Err(LoadError::Io(error))
            if error.kind() == io::ErrorKind::UnexpectedEof)
        );
    }
    let mut trailing = bytes.clone();
    trailing.extend_from_slice(&[1, 2, 3]);
    assert!(matches!(
        Network::from_bytes(&trailing),
        Err(LoadError::Length { .. })
    ));
    let mut reader = Cursor::new(trailing);
    assert!(matches!(
        Network::read_from(&mut reader),
        Err(LoadError::TrailingData)
    ));
    assert_eq!(reader.position(), FILE_BYTES as u64 + 1);
    for offset in [40, HEADER_BYTES, FILE_BYTES / 2, FILE_BYTES - 1] {
        let mut corrupt = bytes.clone();
        corrupt[offset] ^= 1;
        assert!(matches!(
            Network::from_bytes(&corrupt),
            Err(LoadError::Checksum)
        ));
    }
}

#[test]
fn checksum_rejects_a_weight_change_even_when_dimensions_match() {
    let mut bytes = encode(synthetic_network());
    // The first input row: a one-unit change there stays far inside every
    // bound, so only the checksum can object.
    let input = HEADER_BYTES + 2 * HIDDEN_SIZE;
    bytes[input] ^= 1;
    assert!(matches!(
        Network::from_bytes(&bytes),
        Err(LoadError::Checksum)
    ));
    update_checksum(&mut bytes);
    assert!(Network::from_bytes(&bytes).is_ok());
}

#[test]
fn stream_errors_propagate_with_a_source() {
    struct Broken;
    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("injected read error"))
        }
    }
    let error = Network::read_from(Broken).unwrap_err();
    assert!(matches!(error, LoadError::Io(_)));
    assert!(error.source().is_some());
    assert!(error.to_string().contains("injected read error"));
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "jakgro-nnue-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn regular_file_loading_checks_length_and_keeps_models_independent() {
    let scratch = Scratch::new();
    let path = scratch.0.join("network with  spaces.nnue");
    let bytes = encode(synthetic_network());
    std::fs::write(&path, &bytes).unwrap();
    let loaded = Network::load(&path).unwrap();
    let board = Board::default();
    let expected = loaded.evaluate(&board);
    std::fs::write(&path, &bytes[..HEADER_BYTES]).unwrap();
    assert!(matches!(
        Network::load(&path),
        Err(LoadError::Length { .. })
    ));
    assert_eq!(loaded.evaluate(&board), expected);
    assert!(matches!(
        Network::load(scratch.0.join("missing")),
        Err(LoadError::Io(_))
    ));
    assert!(matches!(
        Network::load(&scratch.0),
        Err(LoadError::NotRegularFile) | Err(LoadError::Io(_))
    ));
}

#[test]
fn loading_a_model_does_not_change_engine_defaults_or_search() {
    use crate::engine::{Engine, SearchLimits};
    let engine = Engine::new();
    let limits = SearchLimits {
        depth: Some(3),
        ..SearchLimits::default()
    };
    let before = engine.search(&limits);
    let model = Network::from_bytes(&encode(synthetic_network())).unwrap();
    assert_matches(
        &model,
        &Board::default(),
        &model.accumulator(&Board::default()),
    );
    engine.clear_hash();
    let after = engine.search(&limits);
    assert_eq!(engine.aggression(), 75);
    assert_eq!(engine.threads(), 1);
    assert_eq!(before.best_move(), after.best_move());
    let before = before.info().unwrap();
    let after = after.info().unwrap();
    assert_eq!(before.score(), after.score());
    assert_eq!(before.nodes(), after.nodes());
    assert_eq!(before.pv(), after.pv());
}
