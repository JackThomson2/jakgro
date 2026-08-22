#[allow(dead_code)]
#[path = "../tests/support/search_fixtures.rs"]
mod search_fixtures;

use std::time::{Duration, Instant};

use jakgro::engine::{Engine, Position, SearchLimits, SearchScore, SearchTelemetry};

use search_fixtures::{
    SearchFixture, SearchObservation, assert_expected, assert_legal_pv, parse_fixtures,
    run_fixture_with_limits,
};

const SUITES: &[&str] = &[
    include_str!("../tests/data/tactics.epd"),
    include_str!("../tests/data/defense.epd"),
    include_str!("../tests/data/style.epd"),
    include_str!("../tests/data/transpositions.epd"),
];
const SAMPLES: usize = 7;
const ARMED_DEADLINE: Duration = Duration::from_secs(60);

#[global_allocator]
static GLOBAL_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(Debug, PartialEq, Eq)]
struct NullObservation {
    best_move: Option<String>,
    score: Option<SearchScore>,
    nodes: u64,
    telemetry: SearchTelemetry,
}

fn run_null_fixture(fixture: &SearchFixture, enabled: bool) -> NullObservation {
    let mut engine = Engine::new();
    engine.set_aggression(0);
    engine.set_position(Position::from_fen(&fixture.fen).unwrap());
    let result = engine.search(&SearchLimits {
        depth: Some(5),
        null_move: Some(enabled),
        ..SearchLimits::default()
    });
    NullObservation {
        best_move: result.best_move().map(str::to_owned),
        score: result.info().map(|info| info.score()),
        nodes: result.info().map_or(0, |info| info.nodes()),
        telemetry: result.telemetry(),
    }
}

fn null_pair(fixture: &SearchFixture) -> (u64, u64, u64, u64, f64) {
    let disabled = run_null_fixture(fixture, false);
    let enabled = run_null_fixture(fixture, true);
    assert_eq!(
        enabled.best_move, disabled.best_move,
        "{} changed best move with null pruning",
        fixture.id,
    );
    assert_eq!(
        enabled.score, disabled.score,
        "{} changed objective score with null pruning",
        fixture.id,
    );
    assert_eq!(disabled.telemetry.null_move_attempts(), 0);
    let reduction = if disabled.nodes == 0 {
        0.0
    } else {
        (disabled.nodes as f64 - enabled.nodes as f64) * 100.0 / disabled.nodes as f64
    };
    (
        disabled.nodes,
        enabled.nodes,
        enabled.telemetry.null_move_attempts(),
        enabled.telemetry.null_move_cutoffs(),
        reduction,
    )
}

fn main() {
    println!(
        "id,category,bestmove,score,depth,nodes,node_milliseconds,node_nps,\
         timed_milliseconds,timed_nps,timed_ratio,null_off_nodes,null_on_nodes,\
         null_reduction_percent,null_attempts,null_cutoffs"
    );

    for fixture in SUITES.iter().flat_map(|input| parse_fixtures(input)) {
        let node_limits = SearchLimits {
            nodes: Some(fixture.nodes),
            ..SearchLimits::default()
        };
        let timed_limits = SearchLimits {
            nodes: Some(fixture.nodes),
            move_time: Some(ARMED_DEADLINE),
            ..SearchLimits::default()
        };
        let (observation, node_elapsed, timed_elapsed) =
            measure_pair(&fixture, &node_limits, &timed_limits);
        let (null_off_nodes, null_on_nodes, null_attempts, null_cutoffs, null_reduction) =
            null_pair(&fixture);
        let node_nps = nodes_per_second(observation.nodes, node_elapsed);
        let timed_nps = nodes_per_second(observation.nodes, timed_elapsed);
        let score = match observation.score {
            jakgro::engine::SearchScore::Centipawns(score) => format!("cp {score}"),
            jakgro::engine::SearchScore::Mate(moves) => format!("mate {moves}"),
        };

        println!(
            "{},{},{},{},{},{},{},{},{},{},{:.3},{},{},{:.3},{},{}",
            fixture.id,
            fixture.category,
            observation.best_move,
            score,
            observation.depth,
            observation.nodes,
            node_elapsed.as_millis(),
            node_nps,
            timed_elapsed.as_millis(),
            timed_nps,
            timed_nps as f64 / node_nps.max(1) as f64,
            null_off_nodes,
            null_on_nodes,
            null_reduction,
            null_attempts,
            null_cutoffs,
        );
    }
}
fn measure_pair(
    fixture: &SearchFixture,
    node_limits: &SearchLimits,
    timed_limits: &SearchLimits,
) -> (SearchObservation, Duration, Duration) {
    let expected = run_fixture_with_limits(fixture, node_limits);
    assert_expected(fixture, &expected);
    assert_legal_pv(fixture, &expected);
    let timed_expected = run_fixture_with_limits(fixture, timed_limits);
    assert_eq!(
        timed_expected, expected,
        "{} changed result with an armed deadline",
        fixture.id
    );
    let mut node_elapsed = Vec::with_capacity(SAMPLES);
    let mut timed_elapsed = Vec::with_capacity(SAMPLES);

    for sample in 0..SAMPLES {
        if sample % 2 == 0 {
            node_elapsed.push(measure_once(fixture, node_limits, &expected));
            timed_elapsed.push(measure_once(fixture, timed_limits, &expected));
        } else {
            timed_elapsed.push(measure_once(fixture, timed_limits, &expected));
            node_elapsed.push(measure_once(fixture, node_limits, &expected));
        }
    }

    node_elapsed.sort_unstable();
    timed_elapsed.sort_unstable();
    (
        expected,
        node_elapsed[SAMPLES / 2],
        timed_elapsed[SAMPLES / 2],
    )
}

fn measure_once(
    fixture: &SearchFixture,
    limits: &SearchLimits,
    expected: &SearchObservation,
) -> Duration {
    let started = Instant::now();
    let observation = run_fixture_with_limits(fixture, limits);
    let elapsed = started.elapsed();
    assert_eq!(
        observation, *expected,
        "{} was not deterministic during benchmarking",
        fixture.id
    );
    elapsed
}

fn nodes_per_second(nodes: u64, elapsed: Duration) -> u128 {
    u128::from(nodes) * 1_000_000_000 / elapsed.as_nanos().max(1)
}
