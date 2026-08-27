#[path = "support/search_fixtures.rs"]
mod search_fixtures;

use std::collections::BTreeSet;

use search_fixtures::{
    SearchFixture, assert_expected, assert_legal_pv, parse_fixtures, run_fixture,
};

const SUITES: &[(&str, &str)] = &[
    ("tactical", include_str!("data/tactics.epd")),
    ("defensive", include_str!("data/defense.epd")),
    ("style", include_str!("data/style.epd")),
    ("transposition", include_str!("data/transpositions.epd")),
];

#[test]
fn fixed_node_regressions_are_deterministic_and_legal() {
    for fixture in fixtures() {
        let first = run_fixture(&fixture);
        let second = run_fixture(&fixture);

        assert_expected(&fixture, &first);
        assert_legal_pv(&fixture, &first);
        assert_eq!(second, first, "{} was not deterministic", fixture.id);
    }
}

#[test]
fn warmed_transposition_search_reconstructs_a_pv_tail() {
    let mut engine = jakgro::engine::Engine::new();
    engine.set_aggression(0);
    engine.set_position(
        jakgro::engine::Position::from_fen(
            "r1bq1rk1/ppp2ppp/2n2n2/2b1p3/2B1P3/2NP1N2/PPP2PPP/R1BQ1RK1 w - - 0 8",
        )
        .unwrap(),
    );
    let limits = jakgro::engine::SearchLimits {
        depth: Some(5),
        ..jakgro::engine::SearchLimits::default()
    };

    let cold = engine.search(&limits);
    let cold_info = cold.info().expect("cold search completed no iteration");
    let cold_pv = cold_info.pv().to_vec();
    let warm = engine.search(&limits);
    let warm_info = warm.info().expect("warm search completed no iteration");

    assert!(cold_pv.len() > 1, "cold search did not produce a PV tail");
    assert!(warm_info.pv().len() > 1, "warm search lost its PV tail");
    assert!(warm_info.pv().len() <= cold_pv.len());
    assert_eq!(warm.best_move(), cold.best_move());
    assert_eq!(warm_info.score(), cold_info.score());
    assert_eq!(warm_info.pv(), &cold_pv[..warm_info.pv().len()]);
}

#[test]
fn fixture_ids_are_unique_and_files_match_their_categories() {
    let mut ids = BTreeSet::new();

    for (category, input) in SUITES {
        let fixtures = parse_fixtures(input);
        assert!(!fixtures.is_empty(), "{category} suite is empty");
        for fixture in fixtures {
            assert_eq!(
                &fixture.category, category,
                "{} is miscategorized",
                fixture.id
            );
            assert!(
                ids.insert(fixture.id.clone()),
                "duplicate fixture id: {}",
                fixture.id
            );
        }
    }
}

#[test]
fn static_pruning_is_exercised_in_a_rich_position() {
    let mut engine = jakgro::engine::Engine::new();
    engine.set_aggression(0);
    engine.set_position(
        jakgro::engine::Position::from_fen(
            "r1bq1rk1/ppp2ppp/2n2n2/2b1p3/2B1P3/2NP1N2/PPP2PPP/R1BQ1RK1 w - - 0 8",
        )
        .unwrap(),
    );
    let result = engine.search(&jakgro::engine::SearchLimits {
        depth: Some(5),
        ..jakgro::engine::SearchLimits::default()
    });
    let telemetry = result.telemetry();

    assert!(telemetry.static_pruning_attempts() > 0);
    assert!(telemetry.reverse_futility_cutoffs() + telemetry.futility_pruned_moves() > 0);
}

#[test]
fn capture_history_is_trained_by_misordered_cutoffs() {
    let mut engine = jakgro::engine::Engine::new();
    engine.set_aggression(0);
    engine.set_position(
        jakgro::engine::Position::from_fen(
            "r1bq1rk1/ppp2ppp/2n2n2/2b1p3/2B1P3/2NP1N2/PPP2PPP/R1BQ1RK1 w - - 0 8",
        )
        .unwrap(),
    );
    let result = engine.search(&jakgro::engine::SearchLimits {
        depth: Some(5),
        ..jakgro::engine::SearchLimits::default()
    });

    assert!(result.telemetry().capture_history_updates() > 0);
}

#[test]
fn first_capture_cutoffs_update_capture_history() {
    let mut engine = jakgro::engine::Engine::new();
    engine.set_position(
        jakgro::engine::Position::from_fen("4k3/8/8/8/8/8/4q3/4R1K1 w - - 0 1").unwrap(),
    );
    let result = engine.search(&jakgro::engine::SearchLimits {
        depth: Some(5),
        ..jakgro::engine::SearchLimits::default()
    });
    let telemetry = result.telemetry();

    assert!(telemetry.capture_history_first_move_cutoffs() > 0);
    assert!(telemetry.capture_history_updates() > 0);
}

#[test]
fn selective_search_telemetry_attributes_objective_work() {
    let mut engine = jakgro::engine::Engine::new();
    engine.set_aggression(0);
    engine.set_position(
        jakgro::engine::Position::from_fen(
            "r1bq1rk1/ppp2ppp/2n2n2/2b1p3/2B1P3/2NP1N2/PPP2PPP/R1BQ1RK1 w - - 0 8",
        )
        .unwrap(),
    );
    let result = engine.search(&jakgro::engine::SearchLimits {
        depth: Some(5),
        ..jakgro::engine::SearchLimits::default()
    });
    let telemetry = result.telemetry();

    assert!(telemetry.lmr_attempts() > 0);
    assert!(telemetry.lmr_reductions() > 0);
    assert!(telemetry.lmr_attempts() >= telemetry.lmr_reductions());
    assert!(telemetry.lmr_reductions() >= telemetry.lmr_researches());
    assert!(telemetry.lmr_researches() >= telemetry.lmr_research_fail_highs());
    assert!(telemetry.objective_root_nodes() > 0);
    assert_eq!(telemetry.personality_root_nodes(), 0);
    assert_eq!(telemetry.personality_verifications(), 0);

    assert!(telemetry.aspiration_attempts() > 0);
    assert!(
        telemetry.aspiration_fail_lows() + telemetry.aspiration_fail_highs()
            <= telemetry.aspiration_attempts()
    );
    assert!(telemetry.legal_move_probes() > 0);
    assert!(telemetry.legal_move_probes() < telemetry.quiescence_nodes());
    assert!(telemetry.tt_probes() > 0);
    assert!(telemetry.tt_hits() <= telemetry.tt_probes());
    assert!(telemetry.tt_hash_moves() <= telemetry.tt_hits());
    assert!(telemetry.tt_cutoffs() <= telemetry.tt_hits());
    assert!(telemetry.quiescence_nodes() > 0);
    assert!(telemetry.capture_cutoffs() > 0);
    assert!(telemetry.capture_history_updates() > 0);
    assert!(telemetry.capture_history_updates() <= telemetry.capture_cutoffs());

    assert!(telemetry.capture_history_first_move_cutoffs() <= telemetry.capture_cutoffs());
    assert!(telemetry.lmr_shallow_reductions() <= telemetry.lmr_reductions());
    assert!(telemetry.lmr_shallow_researches() <= telemetry.lmr_shallow_reductions());
    assert_eq!(telemetry.quiescence_pruned_captures(), 0);
    assert_eq!(telemetry.horizon_quiescence_pruned_captures(), 0);
}

#[test]
fn selective_search_telemetry_attributes_personality_work() {
    let mut engine = jakgro::engine::Engine::new();
    engine.set_aggression(100);
    engine.set_position(
        jakgro::engine::Position::from_fen(
            "r1bq1rk1/ppp2ppp/2n2n2/2b1p1NQ/2B1P3/2NP4/PPP2PPP/R4RK1 w - - 0 10",
        )
        .unwrap(),
    );
    let result = engine.search(&jakgro::engine::SearchLimits {
        nodes: Some(20_000),
        ..jakgro::engine::SearchLimits::default()
    });
    let telemetry = result.telemetry();

    assert!(telemetry.objective_root_nodes() > 0);
    assert!(telemetry.personality_root_nodes() > 0);
    assert!(telemetry.personality_verifications() > 0);
    assert!(result.info().is_some());

    assert!(
        telemetry.horizon_quiescence_pruned_captures() <= telemetry.quiescence_pruned_captures()
    );

    assert!(telemetry.horizon_quiescence_pruned_captures() > 0);
}

fn fixtures() -> Vec<SearchFixture> {
    SUITES
        .iter()
        .flat_map(|(_, input)| parse_fixtures(input))
        .collect()
}
