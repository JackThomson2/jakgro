#!/usr/bin/env python3
"""Gate objective strength, same-profile strength, personality cost, and style."""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path
from typing import Any

MATCH_CHANNELS = ("objective", "same_profile", "personality", "baseline_personality")


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{path}: expected a JSON object")
    return value


def validate_match(
    label: str,
    summary_path: Path,
    manifest_path: Path,
    settings: dict[str, Any],
) -> tuple[dict[str, Any], str, str]:
    summary = load_json(summary_path)
    manifest = load_json(manifest_path)
    execution = manifest.get("execution", {})
    inputs = manifest.get("inputs", {})
    candidate = inputs.get("candidate", {})
    baseline = inputs.get("baseline", {})
    if execution.get("status") != "complete" or not execution.get("inputs_unchanged"):
        raise ValueError(f"{label}: match manifest is incomplete or changed")
    games = int(summary.get("games", 0))
    if games != int(execution.get("completed_games", -1)):
        raise ValueError(f"{label}: summary and manifest game counts differ")
    if summary.get("inputs", {}).get("manifest_sha256") != sha256_file(manifest_path):
        raise ValueError(f"{label}: summary is not bound to its manifest")
    if summary.get("engines", {}).get("candidate") != candidate.get("name"):
        raise ValueError(f"{label}: candidate name differs from the manifest")
    if summary.get("engines", {}).get("baseline") != baseline.get("name"):
        raise ValueError(f"{label}: baseline name differs from the manifest")
    if int(candidate.get("aggression", -1)) != int(settings["candidate_aggression"]):
        raise ValueError(f"{label}: candidate aggression differs from the contract")
    if int(baseline.get("aggression", -1)) != int(settings["baseline_aggression"]):
        raise ValueError(f"{label}: baseline aggression differs from the contract")
    return summary, str(candidate.get("sha256", "")), str(baseline.get("sha256", ""))


def match_metrics(
    label: str,
    summary: dict[str, Any],
    settings: dict[str, Any],
) -> dict[str, Any]:
    games = int(summary.get("games", 0))
    elo = float(summary.get("confidence", {}).get("elo", float("-inf")))
    interval = summary.get("confidence", {}).get("elo_ci95", [None, None])
    lower = interval[0] if isinstance(interval, list) and interval else None
    failures: list[str] = []
    if games < int(settings["minimum_games"]):
        failures.append("minimum_games")
    minimum_elo = settings.get("minimum_elo")
    if minimum_elo is not None and elo < float(minimum_elo):
        failures.append("minimum_elo")
    minimum_lower = settings.get("minimum_elo_ci95_lower")
    if minimum_lower is not None and (lower is None or float(lower) < float(minimum_lower)):
        failures.append("minimum_elo_ci95_lower")
    return {
        "games": games,
        "elo": elo,
        "elo_ci95": interval,
        "failures": failures,
        "passed": not failures,
        "label": label,
    }


def forcing_retention(summary: dict[str, Any]) -> float:
    style = summary.get("style", {})
    candidate = float(style.get("candidate", {}).get("forcing_moves_per_100_moves", 0.0))
    baseline = float(style.get("baseline", {}).get("forcing_moves_per_100_moves", 0.0))
    if baseline <= 0.0:
        raise ValueError("same_profile: baseline forcing rate must be positive")
    return candidate / baseline * 100.0


def validate_style(
    path: Path,
    candidate_hash: str,
    baseline_hash: str,
) -> dict[str, Any]:
    summary = load_json(path)
    inputs = summary.get("inputs", {})
    if inputs.get("candidate", {}).get("sha256") != candidate_hash:
        raise ValueError("style: candidate binary hash differs")
    if inputs.get("baseline", {}).get("sha256") != baseline_hash:
        raise ValueError("style: baseline binary hash differs")
    gates = summary.get("gates", {})
    expected = bool(gates.get("candidate_expected_moves", {}).get("passed"))
    controls = bool(gates.get("controls_preserved", {}).get("passed"))
    return {
        "candidate_expected_moves": expected,
        "controls_preserved": controls,
        "passed": expected and controls,
    }


def validate_acceptance(
    path: Path,
    candidate_hash: str,
    maximum_root_loss: int,
) -> dict[str, Any]:
    summary = load_json(path)
    if summary.get("inputs", {}).get("engine", {}).get("sha256") != candidate_hash:
        raise ValueError(f"{path}: candidate binary hash differs")
    positions = summary.get("positions", [])
    observed_loss = max((int(position.get("root_loss_cp", 0)) for position in positions), default=0)
    deterministic = bool(summary.get("passed"))
    return {
        "positions": len(positions),
        "maximum_root_loss_cp": observed_loss,
        "passed": deterministic and observed_loss <= maximum_root_loss,
    }


def validate_efficiency(
    path: Path,
    candidate_hash: str,
    baseline_hash: str,
    minimum_reduction: float,
) -> dict[str, Any]:
    summary = load_json(path)
    inputs = summary.get("inputs", {})
    if inputs.get("candidate", {}).get("sha256") != candidate_hash:
        raise ValueError("efficiency: candidate binary hash differs")
    if inputs.get("baseline", {}).get("sha256") != baseline_hash:
        raise ValueError("efficiency: baseline binary hash differs")
    metrics = summary.get("metrics", {})
    reduction = float(metrics.get("geometric_node_reduction_percent", float("-inf")))
    return {
        "active_positions": int(metrics.get("active_positions", 0)),
        "geometric_node_reduction_percent": reduction,
        "minimum_percent": minimum_reduction,
        "passed": reduction >= minimum_reduction,
    }


def gate_artifacts(
    contract_path: Path,
    match_paths: dict[str, tuple[Path, Path]],
    style_path: Path,
    acceptance_paths: list[Path],
    efficiency_path: Path,
) -> dict[str, Any]:
    contract = load_json(contract_path)
    match_contract = contract.get("matches", {})
    if set(match_contract) != set(MATCH_CHANNELS):
        raise ValueError("contract must define objective, same_profile, and personality matches")

    summaries: dict[str, dict[str, Any]] = {}
    hashes: dict[str, tuple[str, str]] = {}
    matches: dict[str, dict[str, Any]] = {}
    for label in MATCH_CHANNELS:
        summary, candidate_hash, baseline_hash = validate_match(
            label,
            match_paths[label][0],
            match_paths[label][1],
            match_contract[label],
        )
        summaries[label] = summary
        hashes[label] = (candidate_hash, baseline_hash)
        matches[label] = match_metrics(label, summary, match_contract[label])

    candidate_hash, baseline_hash = hashes["objective"]
    if hashes["same_profile"] != (candidate_hash, baseline_hash):
        raise ValueError("objective and same-profile matches use different binaries")
    if hashes["personality"] != (candidate_hash, candidate_hash):
        raise ValueError("personality match must compare two profiles of the candidate binary")
    if hashes["baseline_personality"] != (baseline_hash, baseline_hash):
        raise ValueError("baseline personality match must compare two profiles of the baseline binary")

    personality_delta = (
        matches["personality"]["elo"] - matches["baseline_personality"]["elo"]
    )
    minimum_personality_delta = float(
        contract["personality_comparison"]["minimum_elo_delta"]
    )
    minimum_candidate_elo = float(
        contract["personality_comparison"]["minimum_candidate_elo"]
    )
    personality_comparison = {
        "candidate_elo": matches["personality"]["elo"],
        "baseline_elo": matches["baseline_personality"]["elo"],
        "elo_delta": personality_delta,
        "minimum_elo_delta": minimum_personality_delta,
        "minimum_candidate_elo": minimum_candidate_elo,
        "relative_passed": personality_delta >= minimum_personality_delta,
        "absolute_passed": matches["personality"]["elo"] >= minimum_candidate_elo,
    }
    personality_comparison["passed"] = (
        personality_comparison["relative_passed"]
        and personality_comparison["absolute_passed"]
    )

    minimum_forcing = float(contract["deterministic"]["minimum_forcing_retention_percent"])
    retention = forcing_retention(summaries["same_profile"])
    style = validate_style(style_path, candidate_hash, baseline_hash)
    style["forcing_retention_percent"] = retention
    style["minimum_forcing_retention_percent"] = minimum_forcing
    style["passed"] = style["passed"] and retention >= minimum_forcing

    maximum_loss = int(contract["deterministic"]["maximum_root_loss_cp"])
    acceptance = [
        validate_acceptance(path, candidate_hash, maximum_loss) for path in acceptance_paths
    ]
    minimum_reduction = float(contract["efficiency"]["minimum_node_reduction_percent"])
    efficiency = validate_efficiency(
        efficiency_path,
        candidate_hash,
        baseline_hash,
        minimum_reduction,
    )
    passed = (
        all(match["passed"] for match in matches.values())
        and personality_comparison["passed"]
        and style["passed"]
        and all(result["passed"] for result in acceptance)
        and efficiency["passed"]
    )
    artifact_paths = [
        contract_path,
        style_path,
        efficiency_path,
        *acceptance_paths,
        *(path for pair in match_paths.values() for path in pair),
    ]
    return {
        "schema_version": 1,
        "candidate_sha256": candidate_hash,
        "baseline_sha256": baseline_hash,
        "matches": matches,
        "personality_comparison": personality_comparison,
        "style": style,
        "acceptance": acceptance,
        "efficiency": efficiency,
        "artifacts": {path.name: sha256_file(path) for path in artifact_paths},
        "passed": passed,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--contract", type=Path, required=True)
    for label in MATCH_CHANNELS:
        option = label.replace("_", "-")
        parser.add_argument(f"--{option}-summary", type=Path, required=True)
        parser.add_argument(f"--{option}-manifest", type=Path, required=True)
    parser.add_argument("--style-summary", type=Path, required=True)
    parser.add_argument("--acceptance-summary", type=Path, action="append", required=True)
    parser.add_argument("--efficiency-summary", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()

    try:
        match_paths = {
            label: (
                getattr(args, f"{label}_summary"),
                getattr(args, f"{label}_manifest"),
            )
            for label in MATCH_CHANNELS
        }
        result = gate_artifacts(
            args.contract,
            match_paths,
            args.style_summary,
            args.acceptance_summary,
            args.efficiency_summary,
        )
    except (KeyError, OSError, TypeError, ValueError) as error:
        print(f"gate_strength_personality: {error}", file=sys.stderr)
        return 2
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
