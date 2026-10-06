use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use cozy_chess::Board;
use cozy_chess::util::display_uci_move;

use super::{
    MAX_DTZ, Move, NO_SYZYGY_PATH, TablebaseError, TablebaseSummary, Tablebases, Wdl,
    distance_rank, rank_score,
};
use crate::engine::evaluation::TB_WIN_SCORE;
use crate::engine::{Engine, Position, SearchLimits, SearchScore};

/// Every three-piece table that has a result worth probing.
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/syzygy");

fn load() -> Arc<Tablebases> {
    Tablebases::load(FIXTURES).expect("the fixture tables load")
}

fn board(fen: &str) -> Board {
    Position::from_fen(fen).unwrap().board().clone()
}

fn legal_moves(board: &Board) -> Vec<Move> {
    let mut moves = Vec::new();
    board.generate_moves(|piece_moves| {
        moves.extend(piece_moves);
        false
    });
    moves
}

fn uci(board: &Board, moves: &[Move]) -> Vec<String> {
    let mut text = moves
        .iter()
        .map(|&chess_move| display_uci_move(board, chess_move).to_string())
        .collect::<Vec<_>>();
    text.sort();
    text
}

#[test]
fn a_path_reports_its_tables_by_kind_and_size() {
    assert_eq!(
        load().summary(),
        TablebaseSummary {
            wdl_files: 5,
            dtz_files: 5,
            largest: 3,
        },
    );
}

#[test]
fn paths_without_tables_are_refused_before_the_library_sees_them() {
    assert_eq!(
        Tablebases::load("  ").unwrap_err(),
        TablebaseError::EmptyPath
    );
    assert_eq!(
        Tablebases::load("/nonexistent/tablebase/directory").unwrap_err(),
        TablebaseError::NoTables,
    );
    // A refusal must not leave the library unusable for the next, valid load.
    load();
}

#[test]
fn one_set_serves_every_holder_of_a_path_and_another_path_waits_its_turn() {
    let first = load();
    let second = load();
    assert!(Arc::ptr_eq(&first, &second));

    let other = std::env::temp_dir().join(format!("jakgro-syzygy-{}", std::process::id()));
    fs::create_dir_all(&other).unwrap();
    fs::copy(
        PathBuf::from(FIXTURES).join("KPvK.rtbw"),
        other.join("KPvK.rtbw"),
    )
    .unwrap();
    let refused = Tablebases::load(other.to_str().unwrap());
    fs::remove_dir_all(&other).unwrap();
    assert_eq!(refused.unwrap_err(), TablebaseError::InUse);
}

#[test]
fn probes_agree_with_known_results() {
    let tables = load();
    // Expected values come from python-chess reading the same files.
    for (fen, expected) in [
        ("8/8/8/4k3/8/8/8/K6Q w - - 0 1", Wdl::Win),
        ("8/8/8/4k3/8/8/2R5/4K3 w - - 0 1", Wdl::Win),
        ("8/8/8/4k3/8/8/2R5/4K3 b - - 0 1", Wdl::Loss),
        ("4k3/8/4K3/4P3/8/8/8/8 w - - 0 1", Wdl::Win),
        ("k7/8/K7/P7/8/8/8/8 w - - 0 1", Wdl::Draw),
        ("8/8/3k4/8/3P4/3K4/8/8 w - - 0 1", Wdl::Draw),
        ("8/8/8/4k3/8/8/2B5/4K3 w - - 0 1", Wdl::Draw),
        ("8/8/8/4k3/8/8/2N5/4K3 w - - 0 1", Wdl::Draw),
    ] {
        assert_eq!(tables.probe_wdl(&board(fen)), Some(expected), "{fen}");
    }
}

#[test]
fn positions_the_tables_cannot_answer_have_no_result() {
    let tables = load();
    // Four pieces are more than these tables hold.
    assert_eq!(
        tables.probe_wdl(&board("8/8/8/4k3/8/8/2R1P3/4K3 w - - 0 1")),
        None
    );
    // The tables assume no castling rights.
    assert_eq!(
        tables.probe_wdl(&board("r3k3/8/8/8/8/8/8/4K3 b q - 0 1")),
        None
    );
}

#[test]
fn a_win_ranks_by_how_much_of_the_fifty_move_budget_it_leaves() {
    // A win that fits ranks alike wherever it lies; one that overshoots ranks by
    // how far, so a shorter one is preferred.
    assert_eq!(distance_rank(5, 0, false), MAX_DTZ);
    assert_eq!(distance_rank(99, 0, false), MAX_DTZ);
    assert_eq!(distance_rank(20, 79, false), MAX_DTZ);
    assert_eq!(distance_rank(20, 80, false), MAX_DTZ - 100);
    assert!(distance_rank(30, 80, false) < distance_rank(20, 80, false));
    // A loss ranks alike until the fifty-move rule comes into sight, then a
    // longer one is preferred.
    assert_eq!(distance_rank(-10, 0, false), -MAX_DTZ);
    assert_eq!(distance_rank(-40, 30, false), -MAX_DTZ + 70);
    assert!(distance_rank(-60, 30, false) > distance_rank(-40, 30, false));
    assert_eq!(distance_rank(0, 55, false), 0);
}

#[test]
fn after_a_repetition_wins_rank_by_distance_wherever_they_lie() {
    assert_eq!(distance_rank(5, 0, true), MAX_DTZ - 5);
    assert!(distance_rank(5, 0, true) > distance_rank(20, 0, true));
    // Losses and draws do not care.
    assert_eq!(distance_rank(-10, 0, true), -MAX_DTZ);
    assert_eq!(distance_rank(0, 0, true), 0);
}

#[test]
fn rank_scores_separate_certain_results_from_ones_the_fifty_move_rule_decides() {
    assert_eq!(rank_score(MAX_DTZ, true), TB_WIN_SCORE);
    assert_eq!(rank_score(-MAX_DTZ, true), -TB_WIN_SCORE);
    assert_eq!(rank_score(0, true), 0);
    // A win that the rule cancels is worth a few centipawns.
    let cursed = rank_score(MAX_DTZ - 150, true);
    assert!((1..50).contains(&cursed), "{cursed}");
    let blessed = rank_score(-MAX_DTZ + 150, true);
    assert!((-49..0).contains(&blessed), "{blessed}");
    // Without the rule the same rank is a win.
    assert_eq!(rank_score(MAX_DTZ - 150, false), TB_WIN_SCORE);
}

#[test]
fn root_ranking_keeps_only_the_moves_that_win_in_time() {
    let tables = load();
    // White wins with a rook in 21 plies, and 77 of the 100 the fifty-move rule
    // allows are gone. Every rook move still wins in theory, but only one of the
    // 19 legal moves wins in the time that is left; python-chess derives the same
    // move from the distance files.
    let late = board("1K6/8/8/8/8/4R3/8/1k6 w - - 77 1");
    let ranking = tables
        .rank_root(&late, &legal_moves(&late), true, false)
        .expect("the position is covered");
    assert_eq!(uci(&late, &ranking.moves), ["e3c3"]);
    assert_eq!(ranking.score, TB_WIN_SCORE);

    // With the whole budget left every one of them keeps the win.
    let fresh = board("1K6/8/8/8/8/4R3/8/1k6 w - - 0 1");
    let moves = legal_moves(&fresh);
    let ranking = tables
        .rank_root(&fresh, &moves, true, false)
        .expect("the position is covered");
    assert_eq!(ranking.moves.len(), moves.len());
    assert_eq!(moves.len(), 19);
}

#[test]
fn after_a_repetition_only_the_fastest_win_is_kept() {
    let tables = load();
    // Every move still wins with the whole budget left, so a search could shuffle
    // between them until the game is drawn by repetition; once a position has
    // repeated, only the win in 21 plies (the others take 23 to 29) keeps its rank.
    let fresh = board("1K6/8/8/8/8/4R3/8/1k6 w - - 0 1");
    let ranking = tables
        .rank_root(&fresh, &legal_moves(&fresh), true, true)
        .expect("the position is covered");
    assert_eq!(uci(&fresh, &ranking.moves), ["e3c3"]);
    assert_eq!(ranking.score, TB_WIN_SCORE);
}

#[test]
fn root_ranking_of_a_lost_position_keeps_every_move() {
    let tables = load();
    let lost = board("8/8/8/4k3/8/8/2R5/4K3 b - - 0 1");
    let moves = legal_moves(&lost);
    let ranking = tables
        .rank_root(&lost, &moves, true, false)
        .expect("the position is covered");
    assert_eq!(ranking.moves.len(), moves.len());
    assert_eq!(ranking.score, -TB_WIN_SCORE);
}

#[test]
fn ranking_by_result_alone_keeps_the_moves_that_do_not_throw_the_win_away() {
    let tables = load();
    // The black king attacks e1 and f1, so a rook there is lost and the win with it.
    let position = board("8/8/8/8/8/8/5k1K/7R w - - 60 1");
    let moves = legal_moves(&position);
    let ranks = tables
        .rank_by_result(&position, &moves, true)
        .expect("every reply is covered");
    let kept = moves
        .iter()
        .zip(&ranks)
        .filter(|&(_, &rank)| rank == MAX_DTZ)
        .map(|(&chess_move, _)| chess_move)
        .collect::<Vec<_>>();
    assert_eq!(
        uci(&position, &kept),
        ["h1a1", "h1b1", "h1c1", "h1d1", "h1g1", "h2h3"],
    );
}

#[test]
fn an_engine_without_tablebases_says_so_and_a_failed_load_leaves_none() {
    let mut engine = Engine::new();
    assert_eq!(engine.syzygy_path(), None);
    assert_eq!(engine.syzygy_summary(), TablebaseSummary::default());

    let summary = engine.set_syzygy_path(FIXTURES).unwrap();
    assert_eq!(summary.largest, 3);
    assert_eq!(engine.syzygy_path(), Some(FIXTURES));

    assert_eq!(
        engine.set_syzygy_path("/nonexistent/tablebase/directory"),
        Err(TablebaseError::NoTables),
    );
    assert_eq!(engine.syzygy_path(), None);

    engine.set_syzygy_path(FIXTURES).unwrap();
    assert_eq!(
        engine.set_syzygy_path(NO_SYZYGY_PATH),
        Ok(TablebaseSummary::default())
    );
    assert_eq!(engine.syzygy_path(), None);
}

#[test]
fn settings_are_clamped_to_the_advertised_ranges() {
    let mut engine = Engine::new();
    engine.set_syzygy_probe_depth(0);
    assert_eq!(engine.syzygy_probe_depth(), 1);
    engine.set_syzygy_probe_depth(1_000);
    assert_eq!(engine.syzygy_probe_depth(), 100);
    engine.set_syzygy_probe_limit(99);
    assert_eq!(engine.syzygy_probe_limit(), 7);
    engine.set_syzygy_rule_50(false);
    assert!(!engine.syzygy_rule_50());
}

fn search(engine: &Engine, fen: &str, nodes: u64) -> crate::engine::SearchResult {
    let mut engine = engine.clone();
    engine.set_position(Position::from_fen(fen).unwrap());
    engine.search(&SearchLimits {
        nodes: Some(nodes),
        ..SearchLimits::default()
    })
}

#[test]
fn a_drawn_ending_is_scored_as_a_draw_instead_of_a_phantom_advantage() {
    // White cannot make progress here: the tablebase says draw. The evaluation
    // alone thinks the pawn is a large advantage.
    let fen = "8/8/3k4/8/3P4/3K4/8/8 w - - 0 1";
    let plain = search(&Engine::new(), fen, 20_000);
    let SearchScore::Centipawns(believed) = plain.info().unwrap().score() else {
        panic!("a quiet ending has no mate score");
    };
    assert!(believed > 100, "the evaluation alone says {believed}");

    let mut engine = Engine::new();
    engine.set_syzygy_path(FIXTURES).unwrap();
    let informed = search(&engine, fen, 20_000);
    let info = informed.info().unwrap();
    assert_eq!(info.score(), SearchScore::Centipawns(0));
    assert!(info.tablebase_hits() > 0);
}

#[test]
fn a_won_ending_is_reported_as_a_tablebase_win() {
    let mut engine = Engine::new();
    engine.set_syzygy_path(FIXTURES).unwrap();
    let result = search(&engine, "8/8/8/4k3/8/8/2R5/4K3 w - - 0 1", 20_000);
    let SearchScore::Centipawns(score) = result.info().unwrap().score() else {
        panic!("no mate is in sight within the node budget");
    };
    assert!(score > 19_000, "{score}");
}

#[test]
fn the_search_keeps_a_win_that_only_the_tablebase_can_see_running_out() {
    let mut engine = Engine::new();
    engine.set_syzygy_path(FIXTURES).unwrap();
    let result = search(&engine, "1K6/8/8/8/8/4R3/8/1k6 w - - 77 1", 20_000);
    assert_eq!(result.best_move(), Some("e3c3"));
}
