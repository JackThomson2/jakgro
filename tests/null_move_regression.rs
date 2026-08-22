use jakgro::engine::{Engine, Position, SearchLimits, SearchScore, SearchTelemetry};

const CONTRACT: &str = include_str!("data/null-move-contract.epd");

#[derive(Debug)]
struct ContractPosition {
    id: String,
    fen: String,
    null_allowed: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct Observation {
    best_move: Option<String>,
    score: Option<SearchScore>,
    nodes: u64,
    pv: Vec<String>,
    telemetry: SearchTelemetry,
}

fn contracts() -> Vec<ContractPosition> {
    CONTRACT
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let mut fields = line.split(';').map(str::trim);
            let fen = fields.next().unwrap().to_owned();
            let mut id = None;
            let mut null_allowed = None;
            for field in fields {
                let (key, value) = field.split_once(' ').unwrap();
                match key {
                    "id" => id = Some(value.to_owned()),
                    "null" => null_allowed = Some(value == "allow"),
                    _ => {}
                }
            }
            Some(ContractPosition {
                id: id.unwrap(),
                fen,
                null_allowed: null_allowed.unwrap(),
            })
        })
        .collect()
}

fn observe(fixture: &ContractPosition, null_move: bool) -> Observation {
    let mut engine = Engine::new();
    engine.set_aggression(0);
    engine.set_position(Position::from_fen(&fixture.fen).unwrap());
    let result = engine.search(&SearchLimits {
        depth: Some(5),
        null_move: Some(null_move),
        ..SearchLimits::default()
    });
    let info = result.info();
    Observation {
        best_move: result.best_move().map(str::to_owned),
        score: info.map(|info| info.score()),
        nodes: info.map_or(0, |info| info.nodes()),
        pv: info.map_or_else(Vec::new, |info| info.pv().to_vec()),
        telemetry: result.telemetry(),
    }
}

#[test]
fn verified_null_move_matches_disabled_search_on_contract_positions() {
    let mut allowed_attempts = 0;
    let mut allowed_disabled_nodes = 0;
    let mut allowed_enabled_nodes = 0;
    for fixture in contracts() {
        let disabled = observe(&fixture, false);
        let enabled = observe(&fixture, true);

        assert_eq!(
            enabled.best_move, disabled.best_move,
            "{} changed best move",
            fixture.id
        );
        assert_eq!(
            enabled.score, disabled.score,
            "{} changed objective score",
            fixture.id
        );
        assert_eq!(disabled.telemetry.null_move_attempts(), 0);
        assert_eq!(disabled.telemetry.null_move_cutoffs(), 0);
        assert!(
            enabled.telemetry.null_move_cutoffs() <= enabled.telemetry.null_move_verifications()
        );
        assert!(
            enabled.telemetry.null_move_verifications() <= enabled.telemetry.null_move_attempts()
        );

        if fixture.null_allowed {
            allowed_attempts += enabled.telemetry.null_move_attempts();
            allowed_disabled_nodes += disabled.nodes;
            allowed_enabled_nodes += enabled.nodes;
        }
        if !enabled.pv.is_empty() {
            let mut position = Position::from_fen(&fixture.fen).unwrap();
            position
                .apply_uci_moves(&enabled.pv)
                .unwrap_or_else(|error| panic!("{} returned an illegal PV: {error}", fixture.id));
        }
    }
    assert!(
        allowed_attempts > 0,
        "null pruning never activated on allowed positions"
    );
    assert!(
        allowed_enabled_nodes * 100 <= allowed_disabled_nodes * 95,
        "null pruning did not reduce allowed-position nodes by five percent"
    );
}
