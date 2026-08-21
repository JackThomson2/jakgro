#[path = "../tests/support/search_fixtures.rs"]
mod search_fixtures;

use std::time::Instant;

use search_fixtures::{assert_expected, assert_legal_pv, parse_fixtures, run_fixture};

const SUITES: &[&str] = &[
    include_str!("../tests/data/tactics.epd"),
    include_str!("../tests/data/defense.epd"),
    include_str!("../tests/data/style.epd"),
    include_str!("../tests/data/transpositions.epd"),
];

fn main() {
    println!("id,category,bestmove,score,depth,nodes,milliseconds,nps");

    for fixture in SUITES.iter().flat_map(|input| parse_fixtures(input)) {
        let started = Instant::now();
        let observation = run_fixture(&fixture);
        let elapsed = started.elapsed();
        assert_expected(&fixture, &observation);
        assert_legal_pv(&fixture, &observation);
        let nanos = elapsed.as_nanos().max(1);
        let nps = u128::from(observation.nodes) * 1_000_000_000 / nanos;
        let score = match observation.score {
            jakgro::engine::SearchScore::Centipawns(score) => format!("cp {score}"),
            jakgro::engine::SearchScore::Mate(moves) => format!("mate {moves}"),
        };

        println!(
            "{},{},{},{},{},{},{},{}",
            fixture.id,
            fixture.category,
            observation.best_move,
            score,
            observation.depth,
            observation.nodes,
            elapsed.as_millis(),
            nps
        );
    }
}
