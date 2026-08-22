use jakgro::engine::{Engine, Position, SearchLimits, SearchScore};

const SUITE: &str = include_str!("data/personality.epd");
const MATCH_OPENINGS: &str = include_str!("../tools/data/openings.epd");

#[derive(Debug)]
struct PersonalityFixture {
    id: String,
    fen: String,
    nodes: u64,
    base_move: String,
    tuned_move: String,
}

fn parse_suite() -> Vec<PersonalityFixture> {
    SUITE
        .lines()
        .enumerate()
        .filter_map(|(index, raw)| {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let line_number = index + 1;
            let mut fields = line.split(';').map(str::trim);
            let fen = fields
                .next()
                .filter(|fen| !fen.is_empty())
                .unwrap_or_else(|| panic!("personality fixture line {line_number} has no FEN"));
            let mut id = None;
            let mut nodes = None;
            let mut base_move = None;
            let mut tuned_move = None;
            for field in fields.filter(|field| !field.is_empty()) {
                let (key, value) = field.split_once(' ').unwrap_or_else(|| {
                    panic!("personality fixture line {line_number} has malformed field `{field}`")
                });
                match key {
                    "id" => id = Some(value.to_owned()),
                    "nodes" => {
                        nodes = Some(value.parse::<u64>().unwrap_or_else(|_| {
                            panic!(
                                "personality fixture line {line_number} has invalid nodes `{value}`"
                            )
                        }));
                    }
                    "bm0" => base_move = Some(value.to_owned()),
                    "bm100" => tuned_move = Some(value.to_owned()),
                    _ => panic!(
                        "personality fixture line {line_number} has unsupported field `{key}`"
                    ),
                }
            }
            Some(PersonalityFixture {
                id: id
                    .unwrap_or_else(|| panic!("personality fixture line {line_number} has no id")),
                fen: fen.to_owned(),
                nodes: nodes.unwrap_or_else(|| {
                    panic!("personality fixture line {line_number} has no node budget")
                }),
                base_move: base_move.unwrap_or_else(|| {
                    panic!("personality fixture line {line_number} has no bm0 move")
                }),
                tuned_move: tuned_move.unwrap_or_else(|| {
                    panic!("personality fixture line {line_number} has no bm100 move")
                }),
            })
        })
        .collect()
}

fn search_fixture(fixture: &PersonalityFixture, aggression: u8) -> (String, SearchScore) {
    let mut engine = Engine::new();
    engine.set_aggression(aggression);
    engine.set_position(Position::from_fen(&fixture.fen).unwrap());
    let limits = SearchLimits {
        nodes: Some(fixture.nodes),
        ..SearchLimits::default()
    };

    let first = engine.search(&limits);
    let first_move = first.best_move().unwrap().to_owned();
    let first_score = first.info().unwrap().score();
    engine.clear_hash();
    let second = engine.search(&limits);

    assert_eq!(
        second.best_move(),
        Some(first_move.as_str()),
        "{} was not move-deterministic at Aggression {aggression}",
        fixture.id,
    );
    assert_eq!(
        second.info().unwrap().score(),
        first_score,
        "{} was not score-deterministic at Aggression {aggression}",
        fixture.id,
    );
    (first_move, first_score)
}

#[test]
fn tuned_aggression_profile_is_reproducible_and_distinct() {
    let fixtures = parse_suite();
    assert!(!fixtures.is_empty());
    let mut changed = 0;

    for fixture in &fixtures {
        let (base_move, _) = search_fixture(fixture, 0);
        let (tuned_move, _) = search_fixture(fixture, 100);

        assert_eq!(base_move, fixture.base_move, "{} base profile", fixture.id);
        assert_eq!(
            tuned_move, fixture.tuned_move,
            "{} tuned profile",
            fixture.id
        );
        changed += usize::from(base_move != tuned_move);
    }

    assert_eq!(changed, 4);
    assert_eq!(fixtures.len() - changed, 2);
}
#[test]
fn deterministic_match_openings_are_valid() {
    let mut count = 0;
    for raw in MATCH_OPENINGS.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let epd = line
            .split_once(" id ")
            .map_or(line.trim_end_matches(';'), |(fen, _)| fen);
        Position::from_fen(&format!("{epd} 0 1")).unwrap();
        count += 1;
    }
    assert!(count >= 4);
}
