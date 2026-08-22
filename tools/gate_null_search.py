#!/usr/bin/env python3
"""Gate null-only strength, personality retention, and search benefit."""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import math
import sys
from pathlib import Path
from typing import Any


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def load_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{path}: expected a JSON object")
    return value


def engine_input(manifest: dict[str, Any], role: str) -> dict[str, Any]:
    value = manifest.get("inputs", {}).get(role)
    if not isinstance(value, dict):
        raise ValueError(f"manifest misses {role} input")
    return value


def validate_manifest(summary: dict[str, Any], manifest: dict[str, Any]) -> tuple[str, str]:
    execution = manifest.get("execution", {})
    settings = manifest.get("settings", {})
    if execution.get("status") != "complete" or execution.get("inputs_unchanged") is not True:
        raise ValueError("strength manifest is incomplete or mutable")
    if execution.get("completed_games") != settings.get("games"):
        raise ValueError("strength manifest game count mismatch")
    candidate = engine_input(manifest, "candidate")
    baseline = engine_input(manifest, "baseline")
    if candidate.get("aggression") != 100 or baseline.get("aggression") != 100:
        raise ValueError("strength match is not Aggression 100 versus 100")
    candidate_hash = str(candidate.get("sha256", ""))
    baseline_hash = str(baseline.get("sha256", ""))
    if not candidate_hash or not baseline_hash or candidate_hash == baseline_hash:
        raise ValueError("strength match needs distinct binary hashes")
    engines = summary.get("engines", {})
    if engines.get("candidate") != candidate.get("name"):
        raise ValueError("strength candidate name mismatch")
    if engines.get("baseline") != baseline.get("name"):
        raise ValueError("strength baseline name mismatch")
    if summary.get("games") != settings.get("games"):
        raise ValueError("strength summary game count mismatch")
    return candidate_hash, baseline_hash


def validate_style(
    summary: dict[str, Any],
    candidate_hash: str,
    baseline_hash: str,
    positions: int,
    label: str,
) -> None:
    inputs = summary.get("inputs", {})
    if inputs.get("candidate", {}).get("sha256") != candidate_hash:
        raise ValueError(f"{label} candidate hash mismatch")
    if inputs.get("baseline", {}).get("sha256") != baseline_hash:
        raise ValueError(f"{label} baseline hash mismatch")
    if len(summary.get("positions", [])) != positions:
        raise ValueError(f"{label} position count mismatch")
    gates = summary.get("gates", {})
    if gates.get("candidate_expected_moves", {}).get("passed") is not True:
        raise ValueError(f"{label} expected-move gate failed")
    if gates.get("controls_preserved", {}).get("passed") is not True:
        raise ValueError(f"{label} control gate failed")


def benchmark_gate(path: Path, minimum_reduction: float) -> dict[str, Any]:
    rows = list(csv.DictReader(path.read_text(encoding="utf-8").splitlines()))
    active = [row for row in rows if int(row["null_attempts"]) > 0]
    if not active:
        raise ValueError("benchmark recorded no null attempts")
    ratios = [int(row["null_on_nodes"]) / int(row["null_off_nodes"]) for row in active]
    reduction = (1.0 - math.exp(sum(math.log(ratio) for ratio in ratios) / len(ratios))) * 100.0
    attempts = sum(int(row["null_attempts"]) for row in active)
    cutoffs = sum(int(row["null_cutoffs"]) for row in active)
    if cutoffs > attempts:
        raise ValueError("benchmark has more cutoffs than attempts")
    if reduction < minimum_reduction:
        raise ValueError(
            f"null node reduction {reduction:.3f}% is below {minimum_reduction:.3f}%"
        )
    return {
        "active_rows": len(active),
        "attempts": attempts,
        "cutoffs": cutoffs,
        "geometric_node_reduction_percent": round(reduction, 6),
    }


def gate_artifacts(args: argparse.Namespace) -> dict[str, Any]:
    contract = load_json(args.contract)
    acceptance = contract.get("acceptance", {})
    elo_floor = float(acceptance["same_profile_elo_floor"])
    forcing_floor = float(acceptance["forcing_rate_retention_percent"])
    summary = load_json(args.strength_summary)
    manifest = load_json(args.strength_manifest)
    style = load_json(args.style_summary)
    sacrifice = load_json(args.sacrifice_summary)
    candidate_hash, baseline_hash = validate_manifest(summary, manifest)
    validate_style(style, candidate_hash, baseline_hash, 16, "personality style")
    validate_style(sacrifice, candidate_hash, baseline_hash, 4, "sacrifice style")

    elo = float(summary.get("confidence", {}).get("elo"))
    if elo < elo_floor:
        raise ValueError(f"strength Elo {elo:.3f} is below {elo_floor:.3f}")
    match_style = summary.get("style", {})
    candidate_forcing = float(match_style.get("candidate", {}).get("forcing_moves_per_100_moves"))
    baseline_forcing = float(match_style.get("baseline", {}).get("forcing_moves_per_100_moves"))
    forcing_retention = candidate_forcing * 100.0 / baseline_forcing if baseline_forcing else 0.0
    if forcing_retention < forcing_floor:
        raise ValueError(
            f"forcing-rate retention {forcing_retention:.3f}% is below {forcing_floor:.3f}%"
        )
    benchmark = benchmark_gate(args.benchmark, args.minimum_null_reduction)
    files = [
        args.contract,
        args.strength_summary,
        args.strength_manifest,
        args.style_summary,
        args.sacrifice_summary,
        args.benchmark,
    ]
    return {
        "schema_version": 1,
        "passed": True,
        "candidate_sha256": candidate_hash,
        "baseline_sha256": baseline_hash,
        "strength_elo": elo,
        "strength_elo_floor": elo_floor,
        "strength_elo_interval": summary.get("confidence", {}).get("elo_ci95"),
        "forcing_rate_retention_percent": round(forcing_retention, 6),
        "benchmark": benchmark,
        "artifacts": {path.name: sha256_file(path) for path in files},
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--contract", type=Path, default=Path("tests/data/dual-channel-null-contract.json"))
    parser.add_argument("--strength-summary", type=Path, required=True)
    parser.add_argument("--strength-manifest", type=Path, required=True)
    parser.add_argument("--style-summary", type=Path, required=True)
    parser.add_argument("--sacrifice-summary", type=Path, required=True)
    parser.add_argument("--benchmark", type=Path, required=True)
    parser.add_argument("--minimum-null-reduction", type=float, default=5.0)
    parser.add_argument("--json", type=Path)
    args = parser.parse_args()
    try:
        result = gate_artifacts(args)
        rendered = f"{json.dumps(result, indent=2, sort_keys=True)}\n"
        if args.json is None:
            print(rendered, end="")
        else:
            args.json.parent.mkdir(parents=True, exist_ok=True)
            args.json.write_text(rendered, encoding="utf-8")
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        print(f"gate_null_search: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
