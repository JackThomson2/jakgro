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

fn fixtures() -> Vec<SearchFixture> {
    SUITES
        .iter()
        .flat_map(|(_, input)| parse_fixtures(input))
        .collect()
}
