#[allow(dead_code)]
#[path = "../tests/support/search_fixtures.rs"]
mod search_fixtures;

use std::time::{Duration, Instant};

use jakgro::engine::SearchLimits;

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

fn main() {
    println!(
        "id,category,bestmove,score,depth,nodes,node_milliseconds,node_nps,\
         timed_milliseconds,timed_nps,timed_ratio"
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
        let node_nps = nodes_per_second(observation.nodes, node_elapsed);
        let timed_nps = nodes_per_second(observation.nodes, timed_elapsed);
        let score = match observation.score {
            jakgro::engine::SearchScore::Centipawns(score) => format!("cp {score}"),
            jakgro::engine::SearchScore::Mate(moves) => format!("mate {moves}"),
        };

        println!(
            "{},{},{},{},{},{},{},{},{},{},{:.3}",
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
