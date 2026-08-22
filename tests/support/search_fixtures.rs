use jakgro::engine::{Engine, Position, SearchLimits, SearchScore};

#[derive(Clone, Debug)]
pub struct SearchFixture {
    pub id: String,
    pub category: String,
    pub fen: String,
    pub nodes: u64,
    pub best_move: String,
    pub score: SearchScore,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchObservation {
    pub best_move: String,
    pub score: SearchScore,
    pub depth: u32,
    pub nodes: u64,
    pub pv: Vec<String>,
}

pub fn parse_fixtures(input: &str) -> Vec<SearchFixture> {
    input
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }

            Some(parse_fixture(line, index + 1))
        })
        .collect()
}

pub fn run_fixture(fixture: &SearchFixture) -> SearchObservation {
    let limits = SearchLimits {
        nodes: Some(fixture.nodes),
        ..SearchLimits::default()
    };
    run_fixture_with_limits(fixture, &limits)
}

pub fn run_fixture_with_limits(
    fixture: &SearchFixture,
    limits: &SearchLimits,
) -> SearchObservation {
    let mut engine = Engine::new();
    engine.set_position(
        Position::from_fen(&fixture.fen)
            .unwrap_or_else(|error| panic!("{} has an invalid FEN: {error}", fixture.id)),
    );
    let result = engine.search(limits);
    let info = result
        .info()
        .unwrap_or_else(|| panic!("{} completed no iteration", fixture.id));

    SearchObservation {
        best_move: result
            .best_move()
            .unwrap_or_else(|| panic!("{} produced no best move", fixture.id))
            .to_owned(),
        score: info.score(),
        depth: info.depth(),
        nodes: info.nodes(),
        pv: info.pv().to_vec(),
    }
}

pub fn assert_expected(fixture: &SearchFixture, observation: &SearchObservation) {
    assert_eq!(
        observation.best_move, fixture.best_move,
        "{} selected a different move",
        fixture.id
    );
    assert_eq!(
        observation.score, fixture.score,
        "{} returned a different score",
        fixture.id
    );
    assert!(
        observation.nodes <= fixture.nodes,
        "{} exceeded its node budget: {} > {}",
        fixture.id,
        observation.nodes,
        fixture.nodes
    );
    assert_eq!(
        observation.pv.first(),
        Some(&observation.best_move),
        "{} reported a PV that does not start with bestmove",
        fixture.id
    );
}

pub fn assert_legal_pv(fixture: &SearchFixture, observation: &SearchObservation) {
    let mut position = Position::from_fen(&fixture.fen).expect("fixture FEN was already validated");
    position
        .apply_uci_moves(&observation.pv)
        .unwrap_or_else(|error| panic!("{} reported an illegal PV: {error}", fixture.id));
}

fn parse_fixture(line: &str, line_number: usize) -> SearchFixture {
    let mut fields = line.split(';').map(str::trim);
    let fen = fields
        .next()
        .filter(|field| !field.is_empty())
        .unwrap_or_else(|| panic!("fixture line {line_number} has no FEN"))
        .to_owned();
    let mut id = None;
    let mut category = None;
    let mut nodes = None;
    let mut best_move = None;
    let mut score = None;

    for field in fields {
        let (name, value) = field
            .split_once(' ')
            .unwrap_or_else(|| panic!("fixture line {line_number} has a malformed field: {field}"));
        let value = value.trim();
        match name {
            "id" => id = Some(value.to_owned()),
            "category" => category = Some(value.to_owned()),
            "nodes" => {
                nodes = Some(value.parse().unwrap_or_else(|_| {
                    panic!("fixture line {line_number} has an invalid node budget: {value}")
                }));
            }
            "bm" => best_move = Some(value.to_owned()),
            "score" => score = Some(parse_score(value, line_number)),
            _ => panic!("fixture line {line_number} has an unknown field: {name}"),
        }
    }

    SearchFixture {
        id: id.unwrap_or_else(|| panic!("fixture line {line_number} has no id")),
        category: category.unwrap_or_else(|| panic!("fixture line {line_number} has no category")),
        fen,
        nodes: nodes.unwrap_or_else(|| panic!("fixture line {line_number} has no node budget")),
        best_move: best_move
            .unwrap_or_else(|| panic!("fixture line {line_number} has no best move")),
        score: score.unwrap_or_else(|| panic!("fixture line {line_number} has no score")),
    }
}

fn parse_score(value: &str, line_number: usize) -> SearchScore {
    let (kind, amount) = value
        .split_once(' ')
        .unwrap_or_else(|| panic!("fixture line {line_number} has a malformed score: {value}"));
    let amount = amount
        .parse::<i32>()
        .unwrap_or_else(|_| panic!("fixture line {line_number} has an invalid score: {value}"));
    match kind {
        "cp" => SearchScore::Centipawns(amount),
        "mate" => SearchScore::Mate(amount),
        _ => panic!("fixture line {line_number} has an unknown score kind: {kind}"),
    }
}
