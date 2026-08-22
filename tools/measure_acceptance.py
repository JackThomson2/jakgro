#!/usr/bin/env python3
"""Execute fixed-node strength and personality acceptance contracts over UCI."""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Protocol

try:
    from . import measure_style
except ImportError:
    import measure_style

MOVE = re.compile(r"[a-h][1-8][a-h][1-8][nbrq]?")
MATE_SCORE_CP = 100_000


class MeasurementEngine(Protocol):
    def measure(
        self,
        fixture: measure_style.Fixture,
        aggression: int,
        root_moves: frozenset[str] | None = None,
    ) -> measure_style.Observation: ...

@dataclass(frozen=True)
class ContractFixture:
    identifier: str
    category: str
    fen: str
    nodes: int
    objective_moves: frozenset[str]
    expected: dict[int, frozenset[str]]
    maximum_loss_cp: int
    gate: str
    motif: str

    def search_fixture(self) -> measure_style.Fixture:
        return measure_style.Fixture(
            self.identifier,
            self.category,
            self.fen,
            self.nodes,
            self.expected,
        )


def parse_move_set(path: Path, line_number: int, key: str, value: str) -> frozenset[str]:
    moves = frozenset(move.strip() for move in value.split(",") if move.strip())
    if not moves or any(MOVE.fullmatch(move) is None for move in moves):
        raise ValueError(f"{path}:{line_number}: invalid field {key!r}")
    return moves


def parse_contract_suite(path: Path) -> list[ContractFixture]:
    fixtures: list[ContractFixture] = []
    identifiers: set[str] = set()
    supported = {
        "id",
        "category",
        "nodes",
        "obm",
        "bm0",
        "bm100",
        "maxloss",
        "gate",
        "motif",
    }
    required = {"id", "category", "nodes", "obm", "bm100", "maxloss", "gate", "motif"}

    for line_number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        fields = [field.strip() for field in line.split(";") if field.strip()]
        fen = fields[0]
        if len(fen.split()) != 6:
            raise ValueError(f"{path}:{line_number}: expected a six-field FEN")
        values: dict[str, str] = {}
        for field in fields[1:]:
            key, separator, value = field.partition(" ")
            if not separator or not value.strip():
                raise ValueError(f"{path}:{line_number}: malformed field {field!r}")
            if key not in supported:
                raise ValueError(f"{path}:{line_number}: unsupported field {key!r}")
            if key in values:
                raise ValueError(f"{path}:{line_number}: duplicate field {key!r}")
            values[key] = value.strip()
        missing = sorted(required.difference(values))
        if missing:
            raise ValueError(f"{path}:{line_number}: missing {', '.join(missing)}")
        identifier = values["id"]
        if identifier in identifiers:
            raise ValueError(f"{path}:{line_number}: duplicate id {identifier!r}")
        identifiers.add(identifier)
        try:
            nodes = int(values["nodes"])
            maximum_loss_cp = int(values["maxloss"])
        except ValueError as error:
            raise ValueError(f"{path}:{line_number}: invalid numeric field") from error
        if nodes <= 0:
            raise ValueError(f"{path}:{line_number}: node budget must be positive")
        if not 0 <= maximum_loss_cp <= 120:
            raise ValueError(f"{path}:{line_number}: maxloss must be between 0 and 120")
        expected = {
            profile: parse_move_set(path, line_number, key, values[key])
            for profile, key in ((0, "bm0"), (100, "bm100"))
            if key in values
        }
        fixtures.append(
            ContractFixture(
                identifier=identifier,
                category=values["category"],
                fen=fen,
                nodes=nodes,
                objective_moves=parse_move_set(path, line_number, "obm", values["obm"]),
                expected=expected,
                maximum_loss_cp=maximum_loss_cp,
                gate=values["gate"],
                motif=values["motif"],
            )
        )
    if not fixtures:
        raise ValueError(f"{path}: no fixtures")
    return fixtures


def score_to_cp(score: str) -> int:
    fields = score.split()
    if len(fields) != 2 or fields[0] not in {"cp", "mate"}:
        raise ValueError(f"unsupported UCI score {score!r}")
    try:
        value = int(fields[1])
    except ValueError as error:
        raise ValueError(f"unsupported UCI score {score!r}") from error
    if fields[0] == "cp":
        return value
    distance = min(abs(value), MATE_SCORE_CP - 1)
    return MATE_SCORE_CP - distance if value >= 0 else -MATE_SCORE_CP + distance


def observation_json(observation: measure_style.Observation) -> dict[str, object]:
    return {
        "bestmove": observation.bestmove,
        "score": observation.score,
        "score_cp": score_to_cp(observation.score),
        "depth": observation.depth,
        "nodes": observation.nodes,
    }


def measure_positions(
    engine: MeasurementEngine,
    fixtures: list[ContractFixture],
) -> list[dict[str, Any]]:
    positions: list[dict[str, Any]] = []
    for fixture in fixtures:
        search_fixture = fixture.search_fixture()
        profiles: dict[str, dict[str, object]] = {}
        expected_failures: list[int] = []
        for profile in sorted(fixture.expected):
            observation = engine.measure(search_fixture, profile)
            expected = fixture.expected[profile]
            hit = observation.bestmove in expected
            expected_failures.extend([] if hit else [profile])
            profiles[str(profile)] = {
                **observation_json(observation),
                "expected": sorted(expected),
                "expected_hit": hit,
            }

        selected_move = str(profiles["100"]["bestmove"])
        objective = engine.measure(search_fixture, 0, fixture.objective_moves)
        selected = engine.measure(search_fixture, 0, frozenset({selected_move}))
        objective_cp = score_to_cp(objective.score)
        selected_cp = score_to_cp(selected.score)
        loss_cp = max(0, objective_cp - selected_cp)
        loss_passed = loss_cp <= fixture.maximum_loss_cp
        expected_passed = not expected_failures
        positions.append(
            {
                "id": fixture.identifier,
                "category": fixture.category,
                "gate": fixture.gate,
                "motif": fixture.motif,
                "nodes": fixture.nodes,
                "profiles": profiles,
                "objective": {
                    **observation_json(objective),
                    "allowed_moves": sorted(fixture.objective_moves),
                },
                "selected_under_objective": observation_json(selected),
                "root_loss_cp": loss_cp,
                "maximum_loss_cp": fixture.maximum_loss_cp,
                "expected_moves_passed": expected_passed,
                "root_loss_passed": loss_passed,
                "passed": expected_passed and loss_passed,
            }
        )
    return positions


def summarize(
    positions: list[dict[str, Any]],
    engine_path: Path,
    suite_path: Path,
) -> dict[str, Any]:
    expected_failures = [
        str(position["id"])
        for position in positions
        if not bool(position["expected_moves_passed"])
    ]
    loss_failures = [
        str(position["id"])
        for position in positions
        if not bool(position["root_loss_passed"])
    ]
    control_failures = [
        str(position["id"])
        for position in positions
        if position["gate"] in {"control", "objective"} and not bool(position["passed"])
    ]
    return {
        "schema_version": 1,
        "inputs": {
            "engine": {
                "path": str(engine_path),
                "sha256": measure_style.sha256_file(engine_path),
            },
            "suite": {
                "path": str(suite_path),
                "sha256": measure_style.sha256_file(suite_path),
            },
        },
        "positions": positions,
        "gates": {
            "expected_moves": {
                "passed": not expected_failures,
                "failed_positions": expected_failures,
            },
            "root_loss": {
                "passed": not loss_failures,
                "failed_positions": loss_failures,
            },
            "controls_preserved": {
                "passed": not control_failures,
                "failed_positions": control_failures,
            },
        },
        "passed": not expected_failures and not loss_failures and not control_failures,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument(
        "--suite",
        type=Path,
        default=Path("tests/data/objective-personality-contract.epd"),
    )
    parser.add_argument("--timeout", type=float, default=30.0)
    parser.add_argument("--summary-json", type=Path)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()

    try:
        fixtures = parse_contract_suite(args.suite)
        before_hash = measure_style.sha256_file(args.engine)
        with measure_style.UciEngine(args.engine, args.timeout) as engine:
            positions = measure_positions(engine, fixtures)
        if measure_style.sha256_file(args.engine) != before_hash:
            raise RuntimeError("engine binary changed during measurement")
        summary = summarize(positions, args.engine, args.suite)
    except (OSError, RuntimeError, TimeoutError, ValueError) as error:
        print(f"measure_acceptance: {error}", file=sys.stderr)
        return 2

    for position in positions:
        status = "pass" if position["passed"] else "FAIL"
        selected = position["profiles"]["100"]["bestmove"]
        print(
            f"{position['id']}: {status} bestmove={selected} "
            f"loss={position['root_loss_cp']}/{position['maximum_loss_cp']} cp"
        )
    if args.summary_json:
        args.summary_json.parent.mkdir(parents=True, exist_ok=True)
        args.summary_json.write_text(
            json.dumps(summary, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
    if args.check and not summary["passed"]:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
