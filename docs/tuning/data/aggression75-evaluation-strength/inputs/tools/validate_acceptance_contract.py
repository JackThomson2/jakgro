#!/usr/bin/env python3
"""Validate immutable dual-channel and null-move acceptance inputs."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path
from typing import Any

SHA256 = re.compile(r"[0-9a-f]{64}")
MOVE = re.compile(r"[a-h][1-8][a-h][1-8][nbrq]?")
CONTRACT_KINDS = {
    "objective-personality",
    "sacrifice",
    "null-move",
    "legacy-personality",
    "openings",
}


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def parse_epd(path: Path) -> list[tuple[str, dict[str, str]]]:
    records: list[tuple[str, dict[str, str]]] = []
    identifiers: set[str] = set()
    for line_number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        fields = [field.strip() for field in line.split(";") if field.strip()]
        if len(fields[0].split()) != 6:
            raise ValueError(f"{path}:{line_number}: expected a six-field FEN")
        operations: dict[str, str] = {}
        for field in fields[1:]:
            key, separator, value = field.partition(" ")
            if not separator or not value.strip():
                raise ValueError(f"{path}:{line_number}: malformed operation {field!r}")
            if key in operations:
                raise ValueError(f"{path}:{line_number}: duplicate operation {key}")
            operations[key] = value.strip()
        identifier = operations.get("id")
        if not identifier:
            raise ValueError(f"{path}:{line_number}: missing id")
        if identifier in identifiers:
            raise ValueError(f"{path}:{line_number}: duplicate id {identifier}")
        identifiers.add(identifier)
        records.append((fields[0], operations))
    if not records:
        raise ValueError(f"{path}: suite is empty")
    return records


def parse_moves(path: Path, identifier: str, value: str) -> None:
    moves = value.split(",")
    if not moves or any(MOVE.fullmatch(move) is None for move in moves):
        raise ValueError(f"{path}: {identifier} has invalid move set {value!r}")


def require_fields(path: Path, operations: dict[str, str], fields: set[str]) -> None:
    missing = sorted(fields.difference(operations))
    if missing:
        raise ValueError(f"{path}: {operations.get('id', '?')} misses {', '.join(missing)}")


def validate_epd_suite(path: Path, kind: str) -> int:
    records = parse_epd(path)
    if kind == "legacy-personality":
        return len(records)
    for _, operations in records:
        identifier = operations["id"]
        if kind == "objective-personality":
            require_fields(
                path,
                operations,
                {"category", "nodes", "obm", "bm0", "bm100", "maxloss", "gate", "motif"},
            )
            for field in ("obm", "bm0", "bm100"):
                parse_moves(path, identifier, operations[field])
            if operations["gate"] not in {"objective", "personality", "control", "sacrifice"}:
                raise ValueError(f"{path}: {identifier} has invalid gate")
            maximum = int(operations["maxloss"])
            if not 0 <= maximum <= 120:
                raise ValueError(f"{path}: {identifier} has invalid maxloss")
        elif kind == "sacrifice":
            require_fields(
                path,
                operations,
                {"category", "nodes", "obm", "bm100", "maxloss", "gate", "motif"},
            )
            parse_moves(path, identifier, operations["obm"])
            parse_moves(path, identifier, operations["bm100"])
            if operations["gate"] not in {"positive", "control"}:
                raise ValueError(f"{path}: {identifier} has invalid gate")
            maximum = int(operations["maxloss"])
            if not 0 <= maximum <= 120:
                raise ValueError(f"{path}: {identifier} has invalid maxloss")
        elif kind == "null-move":
            require_fields(path, operations, {"class", "null", "depth", "reason"})
            if operations["null"] not in {"allow", "forbid"}:
                raise ValueError(f"{path}: {identifier} has invalid null policy")
            if int(operations["depth"]) < 1:
                raise ValueError(f"{path}: {identifier} has invalid depth")
    return len(records)


def count_records(path: Path) -> int:
    return sum(
        bool(line.strip()) and not line.lstrip().startswith("#")
        for line in path.read_text(encoding="utf-8").splitlines()
    )


def validate_contract(path: Path, root: Path) -> dict[str, Any]:
    contract = json.loads(path.read_text(encoding="utf-8"))
    if contract.get("schema_version") != 1:
        raise ValueError("unsupported acceptance contract schema")
    baseline = contract.get("baseline_commit")
    if not isinstance(baseline, str) or re.fullmatch(r"[0-9a-f]{40}", baseline) is None:
        raise ValueError("invalid baseline commit")
    suites = contract.get("suites")
    if not isinstance(suites, list) or not suites:
        raise ValueError("acceptance contract has no suites")

    summary: dict[str, int] = {}
    seen_paths: set[str] = set()
    for suite in suites:
        if not isinstance(suite, dict):
            raise ValueError("suite entry must be an object")
        relative = str(suite.get("path", ""))
        kind = str(suite.get("kind", ""))
        expected_hash = str(suite.get("sha256", ""))
        expected_records = int(suite.get("records", -1))
        if relative in seen_paths:
            raise ValueError(f"duplicate suite path {relative}")
        seen_paths.add(relative)
        if kind not in CONTRACT_KINDS:
            raise ValueError(f"unsupported suite kind {kind}")
        if SHA256.fullmatch(expected_hash) is None:
            raise ValueError(f"invalid hash for {relative}")
        suite_path = root / relative
        if sha256_file(suite_path) != expected_hash:
            raise ValueError(f"hash mismatch for {relative}")
        actual_records = (
            validate_epd_suite(suite_path, kind)
            if kind != "openings"
            else count_records(suite_path)
        )
        if actual_records != expected_records:
            raise ValueError(
                f"record count mismatch for {relative}: {actual_records} != {expected_records}"
            )
        summary[kind] = actual_records

    acceptance = contract.get("acceptance")
    if not isinstance(acceptance, dict):
        raise ValueError("acceptance thresholds are missing")
    required_thresholds = {
        "root_loss_cap_cp",
        "personality_required_hits",
        "sacrifice_required_positive_hits",
        "sacrifice_required_control_hits",
        "null_forbidden_attempts",
        "forcing_rate_retention_percent",
        "same_profile_elo_floor",
        "personality_cost_elo_floor",
    }
    if required_thresholds.difference(acceptance):
        raise ValueError("acceptance thresholds are incomplete")
    return {"baseline_commit": baseline, "suites": summary, "acceptance": acceptance}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--contract",
        type=Path,
        default=Path("tests/data/dual-channel-null-contract.json"),
    )
    parser.add_argument("--root", type=Path, default=Path.cwd())
    args = parser.parse_args()
    try:
        summary = validate_contract(args.contract, args.root)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"validate_acceptance_contract: {error}", file=sys.stderr)
        return 2
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
