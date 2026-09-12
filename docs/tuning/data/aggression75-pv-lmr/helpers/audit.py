#!/usr/bin/env python3
"""Audit the retained PV-search matches, identities, statistics and fixtures."""

import argparse
import gzip
import hashlib
import json
import shutil
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[5]
ARCHIVE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from tools import analyze_match, run_sprt, validate_acceptance_contract


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read_json(path):
    return json.loads(path.read_text(encoding="utf-8"))


def fixture_records(path):
    return {fields["id"]: (fen, fields)
            for fen, fields in validate_acceptance_contract.parse_epd(path)}


def audit_fixtures():
    before = ARCHIVE / "fixtures-before"
    after = ARCHIVE / "fixtures-after"
    for path in after.iterdir():
        require(path.read_bytes() == (ROOT / "tests/data" / path.name).read_bytes(),
                f"current fixture differs from archived final input: {path.name}")
    for directory in [before, after]:
        contract = read_json(directory / "dual-channel-null-contract.json")
        for suite in contract["suites"]:
            local = directory / Path(suite["path"]).name
            source = local if local.exists() else ROOT / suite["path"]
            require(sha256(source) == suite["sha256"], f"fixture lock: {source}")
    for name in ["standard-acceptance.epd", "objective-personality-contract.epd", "personality.epd"]:
        old, new = fixture_records(before / name), fixture_records(after / name)
        for identifier, (fen, fields) in old.items():
            new_fen, updated = new[identifier]
            require(fen == new_fen, f"changed fixture position: {identifier}")
            for key in ["nodes", "obm", "maxloss", "gate", "category", "motif"]:
                require(fields.get(key) == updated.get(key), f"changed {key}: {identifier}")
            for key, moves in fields.items():
                if key.startswith("bm"):
                    require(set(moves.split(",")) <= set(updated[key].split(",")),
                            f"removed reviewed alternative: {identifier}/{key}")
            if fields.get("gate") == "control" or fields["category"] in {"safety", "anti-sacrifice"}:
                require(fields == updated, f"relaxed mandatory safety input: {identifier}")
        additions = set(new) - set(old)
        expected = {"standard-opposite-castle-storm-deeper"} if name == "standard-acceptance.epd" else set()
        require(additions == expected, f"unexpected added fixture: {name}")
        if additions:
            _, fields = new[next(iter(additions))]
            require(fields["nodes"] == "400000" and fields["maxloss"] == "0", "deeper storm budget/cap")
            require(all(fields[key] == "b2b4" for key in ["obm", "bm0", "bm75", "bm100"]), "deeper storm target")
    validate_acceptance_contract.validate_contract(after / "dual-channel-null-contract.json", ROOT)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scratch", type=Path, required=True)
    parser.add_argument("--engine", type=Path, help="optionally verify a rebuilt accepted binary")
    parser.add_argument("--baseline", type=Path, help="optionally verify the frozen baseline binary")
    args = parser.parse_args()
    args.scratch.mkdir(parents=True, exist_ok=True)
    require(not args.scratch.resolve().is_relative_to(ARCHIVE), "scratch must be outside the archive")

    index = read_json(ARCHIVE / "sha256.json")
    actual = {str(path.relative_to(ARCHIVE)) for path in ARCHIVE.rglob("*")
              if path.is_file() and path.name != "sha256.json"}
    require(actual == set(index), "artifact inventory differs from the hash index")
    for relative, expected in index.items():
        path = (ARCHIVE / relative).resolve()
        require(path.is_relative_to(ARCHIVE), f"unsafe index path: {relative}")
        require(sha256(path) == expected, f"artifact hash mismatch: {relative}")

    provenance = read_json(ARCHIVE / "provenance.json")
    for relative, expected in provenance["source_inputs"].items():
        require(sha256(ROOT / relative) == expected, f"source binding changed: {relative}")
    for binary, name in [(args.engine, "strength-pv-lmr"), (args.baseline, "strength-base")]:
        if binary is not None:
            require(sha256(binary) == provenance["binary_sha256"][name], f"binary mismatch: {name}")
    books = {}
    for relative, info in provenance["books"].items():
        path = ROOT / relative
        require(sha256(path) == info["sha256"], f"opening book changed: {relative}")
        starts = [" ".join(line.split()[:4]) for line in path.read_text().splitlines()
                  if line.strip() and not line.lstrip().startswith("#")]
        require(len(starts) == info["positions"], f"book count: {relative}")
        require(len(set(starts)) == len(starts), f"duplicate book start: {relative}")
        books[info["sha256"]] = starts

    rows = read_json(ARCHIVE / "matches.json")
    require(len({row["stem"] for row in rows}) == len(rows), "duplicate match ID")
    total_games = 0
    with tempfile.TemporaryDirectory(prefix="pv-lmr-", dir=args.scratch) as temporary:
        temporary = Path(temporary)
        for row in rows:
            stem = row["stem"]
            pgn = temporary / (stem + ".pgn")
            manifest_path = temporary / (stem + ".manifest.json")
            with gzip.open(ARCHIVE / (stem + ".pgn.gz"), "rb") as source, pgn.open("wb") as output:
                shutil.copyfileobj(source, output)
            shutil.copyfile(ARCHIVE / manifest_path.name, manifest_path)
            manifest, candidate, baseline, expected_games = analyze_match.load_manifest(manifest_path, pgn)
            games = analyze_match.parse_pgn(pgn)
            summary = read_json(ARCHIVE / (stem + ".sprt.json"))
            require(summary["status"] == "complete" and not summary["faults"], f"unclean match: {stem}")
            require(not manifest["execution"]["faults"] and manifest["execution"]["return_code"] == 0,
                    f"manifest fault: {stem}")
            require(not run_sprt.arbiter_faults(ARCHIVE / (stem + ".arbiter.json")), f"arbiter fault: {stem}")
            require(summary["inputs"]["manifest_sha256"] == sha256(manifest_path), f"manifest binding: {stem}")
            require(summary["inputs"]["pgn_sha256"] == sha256(pgn), f"PGN binding: {stem}")
            require(manifest["inputs"]["candidate"]["sha256"] == row["candidate_sha256"] == summary["inputs"]["candidate_sha256"], f"candidate binding: {stem}")
            require(manifest["inputs"]["baseline"]["sha256"] == row["baseline_sha256"] == summary["inputs"]["baseline_sha256"] == provenance["binary_sha256"]["strength-base"], f"base binding: {stem}")
            require(manifest["inputs"]["runner"]["sha256"] == provenance["binary_sha256"]["selfplay"], f"runner binding: {stem}")
            require(summary["profiles"] == {"candidate": 75, "baseline": 75}, f"aggression profiles: {stem}")
            opening_hash = summary["inputs"]["openings_sha256"]
            require(opening_hash == manifest["inputs"]["openings"]["sha256"], f"book binding: {stem}")
            starts = books[opening_hash]
            for pair, game in enumerate(games[::2]):
                require(" ".join(game.fen.split()[:4]) == starts[pair], f"opening order: {stem}/{pair}")
            points = run_sprt.pair_points_from_pgn(pgn, candidate, baseline)
            computed = run_sprt.evaluate(points, 0, 10, 0.05, 0.05)
            require(computed == summary["result"] == row["paired_result"], f"paired statistics: {stem}")
            analysis = analyze_match.summarize(games, manifest, candidate, baseline, pgn, manifest_path)
            require(analysis == read_json(ARCHIVE / (stem + "-analysis.json")), f"WDL/style analysis: {stem}")
            require(row["games"] == expected_games == len(games), f"game count: {stem}")
            require(row["wdl"] == analysis["result"] and row["limit"] == summary["limit"], f"match overview: {stem}")
            require(row["hoeffding_elo_ci95"] == analysis["confidence"]["elo_ci95"], f"conservative interval: {stem}")
            a, b = analysis["style"]["candidate"], analysis["style"]["baseline"]
            for output, metric in [("forcing_retention_percent", "forcing_moves_per_100_moves"),
                                   ("check_retention_percent", "checks_per_100_moves")]:
                require(row[output] == round(100 * a[metric] / b[metric], 6), f"retention: {stem}")
            total_games += len(games)
            print(f"{stem}: {len(games)} games, Elo {computed['elo']:+.3f}, {computed['sprt']['decision']}")
            pgn.unlink()
            manifest_path.unlink()
    require(total_games == provenance["games_archived"] == 22432, "total game count")
    require(len(rows) == provenance["matches_archived"] == 13, "total match count")
    audit_fixtures()
    for filename, suite in [("pv-lmr-final-acceptance.json", "standard-acceptance.epd"),
                            ("pv-lmr-final-legacy.json", "objective-personality-contract.epd")]:
        report = read_json(ARCHIVE / filename)
        require(report["passed"] and all(row["passed"] for row in report["positions"]), f"failed final gate: {filename}")
        require(report["inputs"]["engine"]["sha256"] == provenance["binary_sha256"]["strength-pv-lmr"], f"gate engine: {filename}")
        require(report["inputs"]["suite"]["sha256"] == sha256(ARCHIVE / "fixtures-after" / suite), f"gate suite: {filename}")
    endpoints = read_json(ARCHIVE / "pv-lmr-final-endpoints.json")
    require(endpoints["gates"]["candidate_expected_moves"]["passed"] and endpoints["gates"]["controls_preserved"]["passed"], "final endpoint gate")
    require(endpoints["inputs"]["candidate"]["sha256"] == provenance["binary_sha256"]["strength-pv-lmr"], "endpoint engine")
    scope = read_json(ARCHIVE / "pv75-scope-equality.json")
    require(len(scope) == 270 and all(row["equal"] for row in scope), "scope comparison record")
    for row in scope:
        require(all(row["before"][key] == row["after"][key]
                    for key in ["bestmove", "score", "depth", "nodes", "personality"]), "scope comparison content")
    validation = (ARCHIVE / "pv-final-rust-validation.log").read_text()
    require("283 tests run: 283 passed" in validation and "299 tests run: 299 passed" in validation,
            "recorded Rust validation totals")
    require("50 passed" in (ARCHIVE / "pv-final-python-validation.log").read_text(), "recorded Python validation total")
    print(f"Audit passed: {len(index)} indexed files, {len(rows)} matches, {total_games} games; fixtures and source bindings verified.")


if __name__ == "__main__":
    main()
