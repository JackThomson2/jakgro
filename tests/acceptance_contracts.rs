use std::collections::HashSet;

use cozy_chess::{Board, Move};

const OBJECTIVE: &str = include_str!("data/objective-personality-contract.epd");
const SACRIFICE: &str = include_str!("data/sacrifice-acceptance-contract.epd");
const NULL_MOVE: &str = include_str!("data/null-move-contract.epd");

fn operations(line: &str) -> (&str, Vec<(&str, &str)>) {
    let mut fields = line
        .split(';')
        .map(str::trim)
        .filter(|field| !field.is_empty());
    let fen = fields.next().unwrap();
    let operations = fields
        .map(|field| field.split_once(' ').unwrap())
        .collect::<Vec<_>>();
    (fen, operations)
}

fn operation<'a>(operations: &'a [(&str, &str)], key: &str) -> &'a str {
    operations
        .iter()
        .find_map(|(candidate, value)| (*candidate == key).then_some(*value))
        .unwrap_or_else(|| panic!("missing {key}"))
}

#[test]
fn objective_and_sacrifice_contract_moves_are_legal() {
    let mut identifiers = HashSet::new();
    for input in [OBJECTIVE, SACRIFICE] {
        for line in input.lines().filter(|line| {
            let line = line.trim();
            !line.is_empty() && !line.starts_with('#')
        }) {
            let (fen, operations) = operations(line);
            let board: Board = fen.parse().unwrap();
            let identifier = operation(&operations, "id").to_owned();
            assert!(
                identifiers.insert(identifier.clone()),
                "duplicate id: {identifier}"
            );
            for key in ["obm", "bm0", "bm100"] {
                let Some((_, moves)) = operations.iter().find(|(candidate, _)| *candidate == key)
                else {
                    continue;
                };
                for move_text in moves.split(',') {
                    let chess_move: Move = move_text.parse().unwrap();
                    assert!(
                        board.is_legal(chess_move),
                        "{identifier} has illegal {key} move {move_text}"
                    );
                }
            }
            let maximum: i32 = operation(&operations, "maxloss").parse().unwrap();
            assert!((0..=120).contains(&maximum));
        }
    }
}

#[test]
fn null_move_contract_positions_are_valid_and_classified() {
    let mut identifiers = HashSet::new();
    for line in NULL_MOVE.lines().filter(|line| {
        let line = line.trim();
        !line.is_empty() && !line.starts_with('#')
    }) {
        let (fen, operations) = operations(line);
        let identifier = operation(&operations, "id").to_owned();
        let board: Board = fen
            .parse()
            .unwrap_or_else(|error| panic!("{identifier} has invalid FEN: {error:?}"));
        assert!(
            identifiers.insert(identifier.clone()),
            "duplicate id: {identifier}"
        );
        let policy = operation(&operations, "null");
        let reason = operation(&operations, "reason");
        let depth: u32 = operation(&operations, "depth").parse().unwrap();
        assert!(depth >= 1);
        assert!(matches!(policy, "allow" | "forbid"));
        if policy == "allow" {
            assert!(
                board.null_move().is_some(),
                "{identifier} cannot make a null move"
            );
        }
        if reason == "in-check" {
            assert!(board.null_move().is_none(), "{identifier} is not checked");
        }
    }
}
