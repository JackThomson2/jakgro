#!/usr/bin/env python3
"""Compare two UCI engines at a fixed depth across a frozen position suite."""

from __future__ import annotations

import argparse
import json
import math
import sys
from pathlib import Path
from typing import Any, Protocol

try:
    from . import measure_style
except ImportError:
    import measure_style


class DepthEngine(Protocol):
    def measure(
        self,
        fixture: measure_style.Fixture,
        aggression: int,
        root_moves: frozenset[str] | None = None,
        depth: int | None = None,
    ) -> measure_style.Observation: ...


def observation_json(observation: measure_style.Observation) -> dict[str, object]:
    return {
        "bestmove": observation.bestmove,
        "score": observation.score,
        "depth": observation.depth,
        "nodes": observation.nodes,
    }


def measure_rows(
    candidate_engine: DepthEngine,
    baseline_engine: DepthEngine,
    fixtures: list[measure_style.Fixture],
    aggression: int,
    depth: int,
) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for fixture in fixtures:
        expected = fixture.expected.get(aggression, frozenset())
        candidate = candidate_engine.measure(fixture, aggression, depth=depth)
        baseline = baseline_engine.measure(fixture, aggression, depth=depth)
        active = candidate.depth >= depth and baseline.depth >= depth
        rows.append(
            {
                "id": fixture.identifier,
                "category": fixture.category,
                "expected": sorted(expected),
                "candidate": {
                    **observation_json(candidate),
                    "expected_hit": not expected or candidate.bestmove in expected,
                },
                "baseline": {
                    **observation_json(baseline),
                    "expected_hit": not expected or baseline.bestmove in expected,
                },
                "active": active,
                "node_reduction_percent": round(
                    (1.0 - candidate.nodes / baseline.nodes) * 100.0,
                    6,
                )
                if active and baseline.nodes > 0
                else None,
            }
        )
    return rows


def summarize(
    rows: list[dict[str, Any]],
    candidate_path: Path,
    baseline_path: Path,
    suite_path: Path,
    aggression: int,
    depth: int,
    minimum_reduction: float,
) -> dict[str, Any]:
    active = [
        row
        for row in rows
        if row["active"]
        and int(row["candidate"]["nodes"]) > 0
        and int(row["baseline"]["nodes"]) > 0
    ]
    if not active:
        raise ValueError("no positions completed the requested depth")
    log_ratio = sum(
        math.log(int(row["candidate"]["nodes"]) / int(row["baseline"]["nodes"]))
        for row in active
    ) / len(active)
    reduction = (1.0 - math.exp(log_ratio)) * 100.0
    candidate_failures = [
        str(row["id"]) for row in rows if not bool(row["candidate"]["expected_hit"])
    ]
    baseline_failures = [
        str(row["id"]) for row in rows if not bool(row["baseline"]["expected_hit"])
    ]
    return {
        "schema_version": 1,
        "inputs": {
            "candidate": {
                "path": str(candidate_path),
                "sha256": measure_style.sha256_file(candidate_path),
                "aggression": aggression,
            },
            "baseline": {
                "path": str(baseline_path),
                "sha256": measure_style.sha256_file(baseline_path),
                "aggression": aggression,
            },
            "suite": {
                "path": str(suite_path),
                "sha256": measure_style.sha256_file(suite_path),
            },
        },
        "settings": {"depth": depth},
        "positions": rows,
        "metrics": {
            "active_positions": len(active),
            "geometric_node_reduction_percent": round(reduction, 6),
        },
        "gates": {
            "candidate_expected_moves": {
                "passed": not candidate_failures,
                "failed_positions": candidate_failures,
            },
            "baseline_expected_moves": {
                "passed": not baseline_failures,
                "failed_positions": baseline_failures,
            },
            "node_reduction": {
                "passed": reduction >= minimum_reduction,
                "minimum_percent": minimum_reduction,
                "observed_percent": round(reduction, 6),
            },
        },
        "passed": reduction >= minimum_reduction,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--baseline-engine", type=Path, required=True)
    parser.add_argument("--suite", type=Path, default=Path("tests/data/personality.epd"))
    parser.add_argument("--aggression", type=int, default=100)
    parser.add_argument("--depth", type=int, default=4)
    parser.add_argument("--minimum-reduction", type=float, default=0.0)
    parser.add_argument("--timeout", type=float, default=30.0)
    parser.add_argument("--summary-json", type=Path)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()

    if not 0 <= args.aggression <= 100 or args.depth <= 0:
        print("measure_search_efficiency: invalid aggression or depth", file=sys.stderr)
        return 2
    try:
        fixtures = measure_style.parse_suite(args.suite)
        candidate_hash = measure_style.sha256_file(args.engine)
        baseline_hash = measure_style.sha256_file(args.baseline_engine)
        with measure_style.UciEngine(args.engine, args.timeout) as candidate_engine:
            with measure_style.UciEngine(args.baseline_engine, args.timeout) as baseline_engine:
                rows = measure_rows(
                    candidate_engine,
                    baseline_engine,
                    fixtures,
                    args.aggression,
                    args.depth,
                )
        if measure_style.sha256_file(args.engine) != candidate_hash:
            raise RuntimeError("candidate binary changed during measurement")
        if measure_style.sha256_file(args.baseline_engine) != baseline_hash:
            raise RuntimeError("baseline binary changed during measurement")
        summary = summarize(
            rows,
            args.engine,
            args.baseline_engine,
            args.suite,
            args.aggression,
            args.depth,
            args.minimum_reduction,
        )
    except (OSError, RuntimeError, TimeoutError, ValueError) as error:
        print(f"measure_search_efficiency: {error}", file=sys.stderr)
        return 2

    print(
        f"positions={len(rows)} active={summary['metrics']['active_positions']} "
        f"node-reduction={summary['metrics']['geometric_node_reduction_percent']:.3f}%"
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
