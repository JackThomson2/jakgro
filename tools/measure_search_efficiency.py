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


class SearchEngine(Protocol):
    def measure(
        self,
        fixture: measure_style.Fixture,
        aggression: int,
        root_moves: frozenset[str] | None = None,
        depth: int | None = None,
        move_time_ms: int | None = None,
    ) -> measure_style.Observation: ...


def observation_json(observation: measure_style.Observation) -> dict[str, object]:
    return {
        "bestmove": observation.bestmove,
        "score": observation.score,
        "depth": observation.depth,
        "nodes": observation.nodes,
        "elapsed_ms": observation.elapsed_ms,
        "nps": observation.nps,
    }


def measure_rows(
    candidate_engine: SearchEngine,
    baseline_engine: SearchEngine,
    fixtures: list[measure_style.Fixture],
    aggression: int,
    depth: int,
    samples: int = 3,
    move_time_ms: int = 250,
) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for fixture_index, fixture in enumerate(fixtures):
        expected = fixture.expected.get(aggression, frozenset())
        if fixture_index % 2 == 0:
            depth_candidate = candidate_engine.measure(fixture, aggression, depth=depth)
            depth_baseline = baseline_engine.measure(fixture, aggression, depth=depth)
        else:
            depth_baseline = baseline_engine.measure(fixture, aggression, depth=depth)
            depth_candidate = candidate_engine.measure(fixture, aggression, depth=depth)

        candidate_engine.measure(fixture, aggression)
        baseline_engine.measure(fixture, aggression)
        node_candidate_samples: list[measure_style.Observation] = []
        node_baseline_samples: list[measure_style.Observation] = []
        timed_candidate_samples: list[measure_style.Observation] = []
        timed_baseline_samples: list[measure_style.Observation] = []
        for sample in range(samples):
            candidate_first = (fixture_index + sample) % 2 == 0
            if candidate_first:
                node_candidate_samples.append(candidate_engine.measure(fixture, aggression))
                node_baseline_samples.append(baseline_engine.measure(fixture, aggression))
                timed_candidate_samples.append(
                    candidate_engine.measure(
                        fixture, aggression, move_time_ms=move_time_ms
                    )
                )
                timed_baseline_samples.append(
                    baseline_engine.measure(
                        fixture, aggression, move_time_ms=move_time_ms
                    )
                )
            else:
                node_baseline_samples.append(baseline_engine.measure(fixture, aggression))
                node_candidate_samples.append(candidate_engine.measure(fixture, aggression))
                timed_baseline_samples.append(
                    baseline_engine.measure(
                        fixture, aggression, move_time_ms=move_time_ms
                    )
                )
                timed_candidate_samples.append(
                    candidate_engine.measure(
                        fixture, aggression, move_time_ms=move_time_ms
                    )
                )

        node_candidate = median_observation(
            node_candidate_samples, lambda item: item.nps
        )
        node_baseline = median_observation(
            node_baseline_samples, lambda item: item.nps
        )
        timed_candidate = median_observation(
            timed_candidate_samples, lambda item: (item.depth, item.nodes)
        )
        timed_baseline = median_observation(
            timed_baseline_samples, lambda item: (item.depth, item.nodes)
        )
        active = depth_candidate.depth >= depth and depth_baseline.depth >= depth
        reduction = (
            percent_reduction(depth_candidate.nodes, depth_baseline.nodes)
            if active and depth_baseline.nodes > 0
            else None
        )
        nps_gain = (
            percent_gain(node_candidate.nps, node_baseline.nps)
            if node_baseline.nps > 0
            else None
        )
        rows.append(
            {
                "id": fixture.identifier,
                "category": fixture.category,
                "expected": sorted(expected),
                "candidate": {
                    **observation_json(depth_candidate),
                    "expected_hit": not expected or depth_candidate.bestmove in expected,
                },
                "baseline": {
                    **observation_json(depth_baseline),
                    "expected_hit": not expected or depth_baseline.bestmove in expected,
                },
                "active": active,
                "node_reduction_percent": round(reduction, 6)
                if reduction is not None
                else None,
                "fixed_nodes": {
                    "limit": fixture.nodes,
                    "candidate": observation_json(node_candidate),
                    "baseline": observation_json(node_baseline),
                    "candidate_samples": [
                        observation_json(item) for item in node_candidate_samples
                    ],
                    "baseline_samples": [
                        observation_json(item) for item in node_baseline_samples
                    ],
                    "nps_gain_percent": round(nps_gain, 6)
                    if nps_gain is not None
                    else None,
                },
                "timed": {
                    "move_time_ms": move_time_ms,
                    "candidate": observation_json(timed_candidate),
                    "baseline": observation_json(timed_baseline),
                    "candidate_samples": [
                        observation_json(item) for item in timed_candidate_samples
                    ],
                    "baseline_samples": [
                        observation_json(item) for item in timed_baseline_samples
                    ],
                    "depth_gain": timed_candidate.depth - timed_baseline.depth,
                },
            }
        )
    return rows


def median_observation(
    observations: list[measure_style.Observation], key: Any
) -> measure_style.Observation:
    if not observations:
        raise ValueError("at least one sample is required")
    return sorted(observations, key=key)[len(observations) // 2]


def geometric_mean(values: list[float]) -> float:
    if not values:
        return 1.0
    return math.exp(sum(math.log(value) for value in values) / len(values))


def percent_reduction(candidate: int, baseline: int) -> float:
    return (1.0 - candidate / baseline) * 100.0


def percent_gain(candidate: int, baseline: int) -> float:
    return (candidate / baseline - 1.0) * 100.0


def summarize(
    rows: list[dict[str, Any]],
    candidate_path: Path,
    baseline_path: Path,
    suite_path: Path,
    aggression: int,
    depth: int,
    minimum_reduction: float,
    minimum_nps_gain: float = -2.0,
    minimum_depth_gain: float = -0.25,
    samples: int = 3,
    move_time_ms: int = 250,
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
    node_ratios = [
        int(row["candidate"]["nodes"]) / int(row["baseline"]["nodes"])
        for row in active
    ]
    throughput = [
        row
        for row in rows
        if int(row["fixed_nodes"]["candidate"]["nps"]) > 0
        and int(row["fixed_nodes"]["baseline"]["nps"]) > 0
    ]
    if not throughput:
        raise ValueError("no positions reported fixed-node throughput")
    nps_ratios = [
        int(row["fixed_nodes"]["candidate"]["nps"])
        / int(row["fixed_nodes"]["baseline"]["nps"])
        for row in throughput
    ]
    reduction = (1.0 - geometric_mean(node_ratios)) * 100.0
    nps_ratio = geometric_mean(nps_ratios)
    nps_gain = (nps_ratio - 1.0) * 100.0
    depth_gains = [float(row["timed"]["depth_gain"]) for row in rows]
    depth_gain = sum(depth_gains) / len(depth_gains)
    candidate_failures = [
        str(row["id"]) for row in rows if not bool(row["candidate"]["expected_hit"])
    ]
    baseline_failures = [
        str(row["id"]) for row in rows if not bool(row["baseline"]["expected_hit"])
    ]
    passed = (
        not candidate_failures
        and not baseline_failures
        and reduction >= minimum_reduction
        and nps_gain >= minimum_nps_gain
        and depth_gain >= minimum_depth_gain
    )
    return {
        "schema_version": 1,
        "inputs": {
            "candidate": {
                "path": str(candidate_path.resolve()),
                "sha256": measure_style.sha256_file(candidate_path),
                "aggression": aggression,
            },
            "baseline": {
                "path": str(baseline_path.resolve()),
                "sha256": measure_style.sha256_file(baseline_path),
                "aggression": aggression,
            },
            "suite": {
                "path": str(suite_path.resolve()),
                "sha256": measure_style.sha256_file(suite_path),
            },
            "settings": {
                "aggression": aggression,
                "depth": depth,
                "samples": samples,
                "move_time_ms": move_time_ms,
            },
        },
        "settings": {
            "depth": depth,
            "samples": samples,
            "move_time_ms": move_time_ms,
        },
        "metrics": {
            "positions": len(rows),
            "active_positions": len(active),
            "throughput_positions": len(throughput),
            "geometric_candidate_to_baseline_node_ratio": round(
                geometric_mean(node_ratios), 8
            ),
            "geometric_node_reduction_percent": round(reduction, 6),
            "geometric_candidate_to_baseline_nps_ratio": round(nps_ratio, 8),
            "geometric_nps_gain_percent": round(nps_gain, 6),
            "mean_completed_depth_gain": round(depth_gain, 6),
            "candidate_expected_move_failures": candidate_failures,
            "baseline_expected_move_failures": baseline_failures,
        },
        "positions": rows,
        "thresholds": {
            "geometric_node_reduction_percent": {
                "minimum_percent": minimum_reduction,
                "observed_percent": round(reduction, 6),
            },
            "geometric_nps_gain_percent": {
                "minimum_percent": minimum_nps_gain,
                "observed_percent": round(nps_gain, 6),
            },
            "mean_completed_depth_gain": {
                "minimum_plies": minimum_depth_gain,
                "observed_plies": round(depth_gain, 6),
            },
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
            "nps_gain": {
                "passed": nps_gain >= minimum_nps_gain,
                "minimum_percent": minimum_nps_gain,
                "observed_percent": round(nps_gain, 6),
            },
            "completed_depth": {
                "passed": depth_gain >= minimum_depth_gain,
                "minimum_plies": minimum_depth_gain,
                "observed_plies": round(depth_gain, 6),
            },
        },
        "passed": passed,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--baseline-engine", type=Path, required=True)
    parser.add_argument(
        "--suite", type=Path, default=Path("tests/data/search-performance.epd")
    )
    parser.add_argument("--aggression", type=int, default=100)
    parser.add_argument("--depth", type=int, default=4)
    parser.add_argument("--samples", type=int, default=3)
    parser.add_argument("--move-time-ms", type=int, default=250)
    parser.add_argument("--minimum-reduction", type=float, default=0.0)
    parser.add_argument("--minimum-nps-gain", type=float, default=-2.0)
    parser.add_argument("--minimum-depth-gain", type=float, default=-0.25)
    parser.add_argument("--timeout", type=float, default=30.0)
    parser.add_argument("--summary-json", type=Path)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()

    if (
        not 0 <= args.aggression <= 100
        or args.depth <= 0
        or args.samples <= 0
        or args.move_time_ms <= 0
    ):
        print("measure_search_efficiency: invalid measurement settings", file=sys.stderr)
        return 2
    try:
        fixtures = measure_style.parse_suite(args.suite)
        candidate_hash = measure_style.sha256_file(args.engine)
        baseline_hash = measure_style.sha256_file(args.baseline_engine)
        with measure_style.UciEngine(args.engine, args.timeout) as candidate_engine:
            with measure_style.UciEngine(
                args.baseline_engine, args.timeout
            ) as baseline_engine:
                rows = measure_rows(
                    candidate_engine,
                    baseline_engine,
                    fixtures,
                    args.aggression,
                    args.depth,
                    args.samples,
                    args.move_time_ms,
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
            args.minimum_nps_gain,
            args.minimum_depth_gain,
            args.samples,
            args.move_time_ms,
        )
    except (OSError, RuntimeError, TimeoutError, ValueError) as error:
        print(f"measure_search_efficiency: {error}", file=sys.stderr)
        return 2

    print(
        f"positions={len(rows)} active={summary['metrics']['active_positions']} "
        f"node-reduction={summary['metrics']['geometric_node_reduction_percent']:.3f}% "
        f"nps-gain={summary['metrics']['geometric_nps_gain_percent']:.3f}% "
        f"depth-gain={summary['metrics']['mean_completed_depth_gain']:.3f}"
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
