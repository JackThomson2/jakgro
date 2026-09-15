use std::sync::Arc;
use std::time::Duration;

use super::{Engine, NnueConfigError};
use crate::engine::nnue_test_support::NetworkFile;
use crate::engine::{Position, SearchControl, SearchLimits, SearchResult, SearchScore};

fn limits() -> SearchLimits {
    SearchLimits {
        depth: Some(1),
        ..SearchLimits::default()
    }
}

fn configure(file: &NetworkFile) -> Engine {
    let mut engine = Engine::new();
    engine.set_hash_size_mib(1).unwrap();
    engine.set_aggression(0);
    engine.load_eval_file(&file.path).unwrap();
    engine.set_use_nnue(true).unwrap();
    engine
}

fn signature(result: &SearchResult) -> (Option<&str>, SearchScore, u32, u64, &[String]) {
    let info = result.info().unwrap();
    (
        result.best_move(),
        info.score(),
        info.depth(),
        info.nodes(),
        info.pv(),
    )
}

fn same_configuration(first: &Engine, second: &Engine) {
    assert_eq!(first.position(), second.position());
    assert_eq!(first.eval_file(), second.eval_file());
    assert_eq!(first.use_nnue(), second.use_nnue());
    assert_eq!(first.aggression(), second.aggression());
    assert_eq!(first.threads(), second.threads());
    assert_eq!(first.move_overhead(), second.move_overhead());
    assert!(Arc::ptr_eq(&first.table, &second.table));
    assert!(Arc::ptr_eq(&first.memory, &second.memory));
    match (&first.neural.network, &second.neural.network) {
        (Some(a), Some(b)) => assert!(Arc::ptr_eq(a, b)),
        (None, None) => {}
        _ => panic!("loaded model changed"),
    }
}

#[test]
fn the_embedded_network_is_the_default_and_loading_a_file_keeps_the_backend_choice() {
    let mut engine = Engine::new();
    assert!(engine.use_nnue());
    assert!(engine.eval_file().is_none());
    let original = engine.clone();
    engine.set_use_nnue(true).unwrap();
    same_configuration(&engine, &original);
    // Disabling switches to the handcrafted evaluator without dropping the model.
    engine.set_use_nnue(false).unwrap();
    assert!(!engine.use_nnue());
    assert!(!Arc::ptr_eq(&engine.table, &original.table));
    let handcrafted = engine.search(&limits());
    let file = NetworkFile::new(137, false);
    engine.load_eval_file(&file.path).unwrap();
    assert!(!engine.use_nnue());
    assert_eq!(engine.eval_file(), Some(file.path.as_path()));
    let still_handcrafted = engine.search(&limits());
    assert_eq!(signature(&handcrafted), signature(&still_handcrafted));
    // Returning to the embedded network is a real model change.
    let file_table = Arc::clone(&engine.table);
    engine.load_embedded_eval_file().unwrap();
    assert!(engine.eval_file().is_none());
    assert!(!Arc::ptr_eq(&engine.table, &file_table));
    engine.load_embedded_eval_file().unwrap();
    assert!(engine.eval_file().is_none());
    engine.set_use_nnue(true).unwrap();
    let embedded = engine.search(&limits());
    assert_eq!(
        signature(&embedded),
        signature(&original.clone().search(&limits()))
    );
    assert_eq!(engine.aggression(), 75);
}

#[test]
fn loading_errors_preserve_settings_model_position_and_cache_ownership() {
    let file = NetworkFile::new(137, false);
    let invalid = NetworkFile::new(271, false);
    let mut engine = configure(&file);
    engine.set_aggression(37);
    engine.set_threads(2);
    engine.set_move_overhead(Duration::from_millis(41));
    engine.set_position(Position::from_fen("4k3/8/8/8/8/8/4P3/4K3 w - - 0 1").unwrap());
    let before = engine.clone();
    assert!(matches!(
        engine.load_eval_file(""),
        Err(NnueConfigError::EmptyPath)
    ));
    same_configuration(&engine, &before);
    assert!(
        engine
            .load_eval_file(file.path.with_extension("missing"))
            .is_err()
    );
    same_configuration(&engine, &before);
    let valid_bytes = std::fs::read(&invalid.path).unwrap();
    for bytes in [valid_bytes[..48].to_vec(), {
        let mut bytes = valid_bytes.clone();
        bytes[60] ^= 1;
        bytes
    }] {
        std::fs::write(&invalid.path, bytes).unwrap();
        assert!(matches!(
            engine.load_eval_file(&invalid.path),
            Err(NnueConfigError::Load(_))
        ));
        same_configuration(&engine, &before);
    }
    assert_eq!(
        engine
            .neural
            .active_network()
            .unwrap()
            .evaluate(engine.position().board()),
        137
    );
}

#[test]
fn model_and_backend_switches_create_isolated_search_domains() {
    let first_file = NetworkFile::new(137, false);
    let second_file = NetworkFile::new(271, false);
    let mut engine = configure(&first_file);
    let first = engine.search(&limits());
    assert_eq!(first.info().unwrap().score(), SearchScore::Centipawns(-137));
    let old = engine.clone();
    engine.load_eval_file(&second_file.path).unwrap();
    assert!(engine.use_nnue());
    assert!(!Arc::ptr_eq(&engine.table, &old.table));
    assert!(!Arc::ptr_eq(&engine.memory, &old.memory));
    assert!(!engine.has_search_memory());
    assert_eq!(engine.hash_size_mib(), 1);
    let changed = engine.search(&limits());
    assert_eq!(
        changed.info().unwrap().score(),
        SearchScore::Centipawns(-271)
    );
    let fresh = configure(&second_file);
    assert_eq!(signature(&changed), signature(&fresh.search(&limits())));
    assert_eq!(
        old.search(&limits()).info().unwrap().score(),
        SearchScore::Centipawns(-137)
    );
    let neural_clone = engine.clone();
    engine.set_use_nnue(false).unwrap();
    assert_eq!(engine.eval_file(), Some(second_file.path.as_path()));
    assert!(!Arc::ptr_eq(&engine.table, &neural_clone.table));
    let mut handcrafted = Engine::new();
    handcrafted.set_aggression(0);
    handcrafted.set_hash_size_mib(1).unwrap();
    handcrafted.set_use_nnue(false).unwrap();
    assert_eq!(
        signature(&engine.search(&limits())),
        signature(&handcrafted.search(&limits()))
    );
    engine.set_use_nnue(true).unwrap();
    assert_eq!(
        engine.search(&limits()).info().unwrap().score(),
        SearchScore::Centipawns(-271)
    );
    let unchanged = engine.clone();
    engine.set_use_nnue(true).unwrap();
    same_configuration(&engine, &unchanged);
}

#[test]
fn divergent_clones_can_search_concurrently_without_mixing_models() {
    let first_file = NetworkFile::new(137, false);
    let second_file = NetworkFile::new(271, false);
    let mut engine = configure(&first_file);
    let old = engine.clone();
    engine.load_eval_file(&second_file.path).unwrap();
    std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            for _ in 0..3 {
                assert_eq!(
                    old.search(&limits()).info().unwrap().score(),
                    SearchScore::Centipawns(-137)
                );
            }
        });
        let second = scope.spawn(|| {
            for _ in 0..3 {
                assert_eq!(
                    engine.search(&limits()).info().unwrap().score(),
                    SearchScore::Centipawns(-271)
                );
            }
        });
        first.join().unwrap();
        second.join().unwrap();
    });
    engine.set_hash_size_mib(2).unwrap();
    assert_eq!(old.hash_size_mib(), 1);
    assert_eq!(engine.hash_size_mib(), 2);
    let current_clone = engine.clone();
    engine.set_hash_size_mib(3).unwrap();
    assert_eq!(current_clone.hash_size_mib(), 3);
}

#[test]
fn new_game_aggression_and_hash_controls_preserve_the_model() {
    let file = NetworkFile::new(137, false);
    let mut engine = configure(&file);
    let model = Arc::clone(engine.neural.network.as_ref().unwrap());
    engine.set_aggression(75);
    engine.set_threads(2);
    engine.set_move_overhead(Duration::from_millis(41));
    let mut position = Position::default();
    position.apply_uci_moves(["e2e4", "e7e5"]).unwrap();
    engine.set_position(position);
    engine.new_game();
    assert_eq!(engine.position(), &Position::default());
    assert_eq!(engine.eval_file(), Some(file.path.as_path()));
    assert!(engine.use_nnue());
    assert_eq!(engine.aggression(), 75);
    assert_eq!(engine.threads(), 2);
    assert_eq!(engine.move_overhead(), Duration::from_millis(41));
    assert!(Arc::ptr_eq(engine.neural.network.as_ref().unwrap(), &model));
    engine.clear_hash();
    engine.set_hash_size_mib(2).unwrap();
    assert!(Arc::ptr_eq(engine.neural.network.as_ref().unwrap(), &model));
}

#[test]
fn nnue_search_respects_terminal_draw_mate_and_cancellation_rules() {
    let file = NetworkFile::new(137, false);
    let mut engine = configure(&file);
    for fen in [
        "7k/8/8/8/8/8/8/K7 w - - 0 1",
        "7k/8/8/8/8/8/P7/K7 w - - 100 1",
    ] {
        engine.set_position(Position::from_fen(fen).unwrap());
        let result = engine.search(&limits());
        assert_eq!(result.info().unwrap().score(), SearchScore::Centipawns(0));
    }
    engine.set_position(Position::from_fen("7k/6Q1/6K1/8/8/8/8/8 b - - 0 1").unwrap());
    assert_eq!(engine.search(&limits()).best_move(), None);
    engine.set_position(Position::from_fen("7k/5K2/6Q1/8/8/8/8/8 b - - 0 1").unwrap());
    assert_eq!(engine.search(&limits()).best_move(), None);
    engine.set_position(Position::default());
    let control = SearchControl::new();
    control.stop();
    let result = engine.search_with_reporter(&limits(), &control, |_| {
        panic!("pre-stopped search reported")
    });
    assert!(
        engine
            .position()
            .legal_moves()
            .contains(&result.best_move().unwrap().to_owned())
    );
}

#[test]
fn nnue_parallel_workers_return_legal_playable_principal_variations() {
    let file = NetworkFile::new(0, true);
    let mut engine = configure(&file);
    engine.set_threads(3);
    engine.set_aggression(75);
    let result = engine.search(&SearchLimits {
        depth: Some(3),
        ..SearchLimits::default()
    });
    assert!(
        engine
            .position()
            .legal_moves()
            .contains(&result.best_move().unwrap().to_owned())
    );
    let mut replay = engine.position().clone();
    replay.apply_uci_moves(result.info().unwrap().pv()).unwrap();
    assert!(result.info().unwrap().nodes() > 0);
}
