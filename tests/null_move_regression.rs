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
    let board = fixture.fen.parse::<cozy_chess::Board>().unwrap();
    let depth = if board.checkers().is_empty() { 7 } else { 9 };
    observe_at_depth(fixture, null_move, depth)
}

fn observe_at_depth(fixture: &ContractPosition, null_move: bool, depth: u32) -> Observation {
    let mut engine = Engine::new();
    engine.set_aggression(0);
    engine.set_position(Position::from_fen(&fixture.fen).unwrap());
    let result = engine.search(&SearchLimits {
        depth: Some(depth),
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
fn shallow_check_evasions_keep_the_winning_capture_with_or_without_null() {
    let fixture = contracts()
        .into_iter()
        .find(|fixture| fixture.id == "null-in-check")
        .unwrap();
    for enabled in [false, true] {
        let result = observe_at_depth(&fixture, enabled, 7);
        assert_eq!(result.best_move.as_deref(), Some("a2a1"));
        assert!(matches!(result.score, Some(SearchScore::Centipawns(score)) if score >= 1000));
        let mut board = fixture.fen.parse::<cozy_chess::Board>().unwrap();
        for uci in &result.pv {
            let chess_move = cozy_chess::util::parse_uci_move(&board, uci).unwrap();
            assert!(board.is_legal(chess_move));
            board.play_unchecked(chess_move);
        }
        assert_eq!(board.colors(cozy_chess::Color::Black).len(), 1);
        assert_eq!(
            board
                .colored_pieces(cozy_chess::Color::White, cozy_chess::Piece::Queen)
                .len(),
            1,
        );
    }
}

#[test]
fn null_pruning_respects_its_blocks_and_saves_nodes_where_it_fires() {
    let mut allowed_attempts = 0;
    // The position where null pruning fired most, with its node counts with
    // and without it.
    let mut busiest: Option<(u64, u64, u64)> = None;
    for fixture in contracts() {
        let disabled = observe(&fixture, false);
        let enabled = observe(&fixture, true);

        // Null pruning is a heuristic: a fail-high below the verification depth
        // is trusted, so the choice may differ from an unpruned search. A mate
        // must never be lost or invented, and a centipawn score may not drift
        // by more than a pawn.
        match (enabled.score, disabled.score) {
            (Some(SearchScore::Centipawns(with)), Some(SearchScore::Centipawns(without))) => {
                assert!(
                    (with - without).abs() <= 100,
                    "{} changed objective score: {with} vs {without}",
                    fixture.id
                );
            }
            (with, without) => assert_eq!(with, without, "{} changed objective score", fixture.id),
        }
        assert_eq!(disabled.telemetry.null_move_attempts(), 0);
        assert_eq!(disabled.telemetry.null_move_cutoffs(), 0);
        assert!(enabled.telemetry.null_move_cutoffs() <= enabled.telemetry.null_move_fail_highs());
        assert!(
            enabled.telemetry.null_move_verifications() <= enabled.telemetry.null_move_fail_highs()
        );
        assert!(enabled.telemetry.null_move_fail_highs() <= enabled.telemetry.null_move_attempts());

        // Only positions where null pruning actually fired can measure its
        // benefit. A position marked as allowing null pruning may still make no
        // attempt at this depth, because the policy also requires a static
        // evaluation above beta. The benefit is measured on the position where
        // it fired most: the endings in this suite fire it once or twice over a
        // large tree, and summing them in dilutes the one position that
        // exercises the rule with nodes no null search influenced.
        if fixture.null_allowed && enabled.telemetry.null_move_attempts() > 0 {
            allowed_attempts += enabled.telemetry.null_move_attempts();
            let cutoffs = enabled.telemetry.null_move_cutoffs();
            if busiest.is_none_or(|(most, _, _)| cutoffs > most) {
                busiest = Some((cutoffs, disabled.nodes, enabled.nodes));
            }
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
    let (cutoffs, disabled_nodes, enabled_nodes) =
        busiest.expect("an allowed position attempted null pruning");
    assert!(
        cutoffs > 0,
        "null pruning attempted but never cut on any allowed position"
    );
    assert!(
        enabled_nodes * 100 <= disabled_nodes * 95,
        "null pruning did not reduce nodes by five percent where it fired most: \
         {enabled_nodes} against {disabled_nodes} over {cutoffs} cutoffs"
    );
}
