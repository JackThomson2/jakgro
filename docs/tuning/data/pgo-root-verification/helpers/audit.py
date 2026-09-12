#!/usr/bin/env python3
"""Audit the separate PGO and root-verification experiments without replaying games."""

import argparse
import gzip
import hashlib
import json
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[5]
ARCHIVE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from tools import analyze_match, build_pgo, run_sprt, validate_acceptance_contract


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha256(path):
    return build_pgo.sha256(path)


def read_json(path):
    return json.loads(path.read_text(encoding="utf-8"))


def unpack(source, destination):
    destination.parent.mkdir(parents=True, exist_ok=True)
    with gzip.open(source, "rb") as packed, destination.open("wb") as output:
        shutil.copyfileobj(packed, output)


def before_root_patch(current, patch):
    """Reverse the archived, single-file unified diff in memory with exact context."""
    require(patch.count("diff --git ") == 1, "root patch must contain one file")
    require("--- a/src/engine/search/algorithm.rs\n" in patch, "unexpected root patch source")
    require("+++ b/src/engine/search/algorithm.rs\n" in patch, "unexpected root patch target")
    source = current.splitlines(keepends=True)
    lines = patch.splitlines(keepends=True)
    result = []
    source_cursor = 0
    index = 0
    hunks = 0
    while index < len(lines):
        header = re.match(r"@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@", lines[index])
        if header is None:
            index += 1
            continue
        old_count = int(header.group(2) or 1)
        new_count = int(header.group(4) or 1)
        start = int(header.group(3)) - (1 if new_count else 0)
        require(start >= source_cursor, "overlapping root patch hunks")
        index += 1
        before, after = [], []
        while len(before) < old_count or len(after) < new_count:
            require(index < len(lines), "incomplete root patch hunk")
            line = lines[index]
            index += 1
            require(line[:1] in {" ", "+", "-"}, "invalid root patch line")
            if line[0] in " -":
                before.append(line[1:])
            if line[0] in " +":
                after.append(line[1:])
        require(len(before) == old_count and len(after) == new_count, "root patch hunk length")
        require(source[start:start + new_count] == after, "root patch context differs from current source")
        result.extend(source[source_cursor:start])
        result.extend(before)
        source_cursor = start + new_count
        hunks += 1
    require(hunks > 0, "root patch has no hunks")
    result.extend(source[source_cursor:])
    return "".join(result)


def audit_build(provenance, temporary, profdata):
    directory = ARCHIVE / "pgo-build"
    manifest = read_json(directory / "manifest.json")
    require(manifest["status"] == "complete", "PGO build did not complete")
    original_root = Path(manifest["source_root"])
    old_algorithm = before_root_patch(
        (ROOT / "src/engine/search/algorithm.rs").read_text(),
        (ARCHIVE / "round2-root.patch").read_text(),
    )
    old_hash = hashlib.sha256(old_algorithm.encode()).hexdigest()
    differences = {}
    for relative, expected in manifest["source_inputs"].items():
        current = sha256(ROOT / relative)
        if current != expected:
            require(relative == "src/engine/search/algorithm.rs", f"unexpected PGO source delta: {relative}")
            require(old_hash == expected, "reversed root patch does not reconstruct the PGO source")
            differences[relative] = {"pgo_build": expected, "current": current}
    require(differences == provenance["pgo_source_differences"], "PGO source provenance delta")
    for original, expected in manifest["suite_sha256"].items():
        relative = Path(original).relative_to(original_root)
        require(sha256(ARCHIVE / "inputs" / relative) == expected, f"PGO suite binding: {relative}")
    settings = manifest["settings"]
    training_paths = [ARCHIVE / "inputs" / Path(path).relative_to(original_root)
                      for path in settings["training_suites"]]
    validation_path = ARCHIVE / "inputs" / Path(settings["validation_suite"]).relative_to(original_root)
    training_suite = build_pgo.load_suite(training_paths, settings["training_nodes"])
    validation_suite = build_pgo.load_suite([validation_path], settings["validation_nodes"])
    build_pgo.ensure_disjoint(training_suite, validation_suite)
    training = read_json(directory / "training.json")
    validation = read_json(directory / "validation.json")
    build_pgo.validate_observations(validation["baseline"], validation["optimized"])
    for rows, fixtures in [(training, training_suite), (validation["baseline"], validation_suite)]:
        expected_rows = [(fixture.identifier, fixture.fen, profile, fixture.nodes)
                         for profile in settings["profiles"] for fixture in fixtures]
        require([(row["id"], row["fen"], row["aggression"], row["requested_nodes"]) for row in rows] == expected_rows,
                "PGO workload order or settings mismatch")
        for row in rows:
            require(row["iterations"], "missing completed training/validation iteration")
            last = row["iterations"][-1]
            require(all(last[key] == row[key] for key in ["score", "depth", "nodes"]), "iteration summary mismatch")
            require(last["pv"][:1] == [row["bestmove"]], "iteration/bestmove mismatch")
    require(len(training) == 168 and len(validation["baseline"]) == 30, "PGO workload counts")
    require(sum(len(row["iterations"]) for row in validation["optimized"]) == 238, "PGO validation iteration count")
    for name, key in [("training.json", "training_sha256"), ("validation.json", "validation_sha256")]:
        require(sha256(directory / name) == manifest[key], f"PGO observation hash: {name}")
    merged = temporary / "merged.profdata"
    unpack(directory / "merged.profdata.gz", merged)
    require(sha256(merged) == manifest["profile_sha256"], "merged profile hash")
    raw_files = []
    for name, expected in manifest["raw_profiles"].items():
        require(Path(name).name == name, "unsafe raw profile name")
        target = temporary / "profiles" / name
        unpack(directory / "profiles" / (name + ".gz"), target)
        require(sha256(target) == expected, f"raw profile hash: {name}")
        raw_files.append(target)
    require(raw_files, "no raw profiles retained")
    for stage, role in [("baseline", "pgo_base"), ("optimized", "pgo_candidate")]:
        require(manifest["binaries"][stage]["sha256"] == provenance["binary_sha256"][role], f"PGO binary binding: {stage}")
    require(manifest["output"]["sha256"] == provenance["binary_sha256"]["pgo_candidate"], "published PGO hash")
    if profdata is not None:
        require(sha256(profdata) == manifest["toolchain"]["llvm_profdata_sha256"], "llvm-profdata identity")
        rebuilt = temporary / "remerged.profdata"
        subprocess.run([str(profdata.resolve()), "merge", "-o", str(rebuilt), *map(str, raw_files)], check=True, timeout=120)
        require(sha256(rebuilt) == manifest["profile_sha256"], "remerged profile differs")
        print("Raw profiles remerged byte-identically with the recorded LLVM tool.")
    print("PGO build: 168 training searches, 30 validation pairs, 238 identical iterations; source/profile hashes verified.")


def audit_matches(provenance, temporary):
    books = {}
    for relative, info in provenance["books"].items():
        path = ROOT / relative
        require(sha256(path) == info["sha256"], f"book hash: {relative}")
        starts = [" ".join(line.split()[:4]) for line in path.read_text().splitlines()
                  if line.strip() and not line.lstrip().startswith("#")]
        require(len(starts) == info["positions"] and len(set(starts)) == len(starts), f"book records: {relative}")
        books[info["sha256"]] = starts
    declared = {}
    for group in ["root", "pgo"]:
        protocol = read_json(ARCHIVE / f"round2-{group}-protocol.json")
        for match in protocol["matches"]:
            declared[match["name"]] = (group, match)
    rows = read_json(ARCHIVE / "matches.json")
    require({row["stem"] for row in rows} == set(declared) and len(rows) == len(declared), "match inventory differs from protocols")
    games_total = 0
    for row in rows:
        stem = row["stem"]
        group, protocol = declared[stem]
        require(row["group"] == group, f"mixed comparison attribution: {stem}")
        pgn = temporary / (stem + ".pgn")
        manifest_path = temporary / (stem + ".manifest.json")
        unpack(ARCHIVE / (stem + ".pgn.gz"), pgn)
        shutil.copyfile(ARCHIVE / manifest_path.name, manifest_path)
        manifest, candidate, baseline, expected_games = analyze_match.load_manifest(manifest_path, pgn)
        summary = read_json(ARCHIVE / (stem + ".sprt.json"))
        require(summary["status"] == "complete" and not summary["faults"], f"unclean summary: {stem}")
        require(not manifest["execution"]["faults"] and manifest["execution"]["return_code"] == 0, f"manifest fault: {stem}")
        require(not run_sprt.arbiter_faults(ARCHIVE / (stem + ".arbiter.json")), f"arbiter fault: {stem}")
        require(summary["inputs"]["manifest_sha256"] == sha256(manifest_path), f"manifest hash: {stem}")
        require(summary["inputs"]["pgn_sha256"] == sha256(pgn), f"PGN hash: {stem}")
        for role, suffix in [("candidate", "candidate"), ("baseline", "base")]:
            expected = provenance["binary_sha256"][f"{group}_{suffix}"]
            require(manifest["inputs"][role]["sha256"] == summary["inputs"][role + "_sha256"] == expected,
                    f"{role} binary attribution: {stem}")
            require(manifest["inputs"][role]["aggression"] == 75, f"profile: {stem}")
        require(manifest["inputs"]["runner"]["sha256"] == provenance["binary_sha256"]["selfplay"], f"arbiter identity: {stem}")
        require(manifest["inputs"]["harness"]["sha256"] == provenance["measurement_tool_hashes"]["tools/run_sprt.py"], f"harness identity: {stem}")
        require(manifest["settings"]["hash_mib"] == 16 and manifest["settings"]["concurrency"] == 20, f"match resources: {stem}")
        require(summary["profiles"] == {"candidate": 75, "baseline": 75}, f"summary profile: {stem}")
        require(expected_games == protocol["games"], f"declared game cap: {stem}")
        require(summary["limit"] == manifest["settings"]["limit"], f"time-limit binding: {stem}")
        if "movetime_ms" in protocol:
            require(summary["limit"]["movetime_ms"] == protocol["movetime_ms"], f"movetime: {stem}")
        else:
            require(summary["limit"]["time_control"] == protocol["time_control"], f"clock: {stem}")
        opening_hash = summary["inputs"]["openings_sha256"]
        require(opening_hash == manifest["inputs"]["openings"]["sha256"] == provenance["books"][protocol["book"]]["sha256"], f"opening identity: {stem}")
        games = analyze_match.parse_pgn(pgn)
        for pair, game in enumerate(games[::2]):
            require(" ".join(game.fen.split()[:4]) == books[opening_hash][pair], f"opening order: {stem}/{pair}")
        points = run_sprt.pair_points_from_pgn(pgn, candidate, baseline)
        computed = run_sprt.evaluate(points, 0, 10, 0.05, 0.05)
        require(computed == summary["result"] == row["paired_result"], f"paired statistics: {stem}")
        analysis = analyze_match.summarize(games, manifest, candidate, baseline, pgn, manifest_path)
        require(analysis == read_json(ARCHIVE / (stem + "-analysis.json")), f"WDL and style statistics: {stem}")
        require(row["games"] == len(games) == expected_games and row["wdl"] == analysis["result"], f"overview counts: {stem}")
        require(row["limit"] == summary["limit"] and row["hoeffding_elo_ci95"] == analysis["confidence"]["elo_ci95"], f"overview limits: {stem}")
        a, b = analysis["style"]["candidate"], analysis["style"]["baseline"]
        for output, metric in [("forcing_retention_percent", "forcing_moves_per_100_moves"), ("check_retention_percent", "checks_per_100_moves")]:
            require(row[output] == round(100 * a[metric] / b[metric], 6), f"style retention: {stem}")
        games_total += len(games)
        print(f"{group}: {stem}: {len(games)} games, Elo {computed['elo']:+.3f}, {computed['sprt']['decision']}")
        pgn.unlink()
        manifest_path.unlink()
    require(len(rows) == provenance["expected_match_count"] == 5, "match total")
    require(games_total == provenance["expected_game_count"] == 9120, "game total")


def audit_gates(provenance):
    for relative, expected in provenance["fixture_hashes"].items():
        require(sha256(ROOT / relative) == sha256(ARCHIVE / "inputs" / relative) == expected, f"changed fixture: {relative}")
    validate_acceptance_contract.validate_contract(ARCHIVE / "inputs/tests/data/dual-channel-null-contract.json", ARCHIVE / "inputs")
    for group in ["root", "pgo"]:
        candidate_hash = provenance["binary_sha256"][group + "_candidate"]
        base_hash = provenance["binary_sha256"][group + "_base"]
        for kind, suite in [("acceptance", "standard-acceptance.epd"), ("legacy", "objective-personality-contract.epd")]:
            report = read_json(ARCHIVE / f"round2-{group}-{kind}.json")
            require(report["passed"] and all(row["passed"] for row in report["positions"]), f"failed {group}/{kind}")
            require(report["inputs"]["engine"]["sha256"] == candidate_hash, f"gate binary: {group}/{kind}")
            require(report["inputs"]["suite"]["sha256"] == provenance["fixture_hashes"]["tests/data/" + suite], f"gate suite: {group}/{kind}")
        style = read_json(ARCHIVE / f"round2-{group}-style.json")
        require(style["gates"]["candidate_expected_moves"]["passed"] and style["gates"]["controls_preserved"]["passed"], f"style gates: {group}")
        require(style["inputs"]["candidate"]["sha256"] == candidate_hash and style["inputs"]["baseline"]["sha256"] == base_hash, f"style binary pair: {group}")
        sacrifice = read_json(ARCHIVE / f"round2-{group}-sacrifice.json")
        require(len(sacrifice["positions"]) == 8 and all(row["status"] == "pass" for row in sacrifice["positions"]), f"sacrifice controls: {group}")
        efficiency = read_json(ARCHIVE / f"round2-{group}-efficiency.json")
        require(efficiency["passed"], f"efficiency diagnostics: {group}")
        require(efficiency["inputs"]["candidate"]["sha256"] == candidate_hash and efficiency["inputs"]["baseline"]["sha256"] == base_hash, f"efficiency binary pair: {group}")
        require(efficiency["metrics"]["identical_tree_positions"] == efficiency["metrics"]["repeatable_positions"] == 10, f"fixed-depth comparisons: {group}")
    require("2 tests run: 0 passed, 2 failed" in (ARCHIVE / "round2-root-red-tests.log").read_text(), "missing pre-fix failure evidence")
    require("291 tests run: 291 passed" in (ARCHIVE / "round2-root-tests.log").read_text(), "default test count")
    require("307 tests run: 307 passed" in (ARCHIVE / "round2-root-tuning-tests.log").read_text(), "feature test count")
    require("86 passed, 29 subtests passed" in (ARCHIVE / "round2-python-validation.log").read_text(), "Python test count")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scratch", type=Path, required=True)
    parser.add_argument("--llvm-profdata", type=Path, help="optionally remerge using the exact recorded tool")
    parser.add_argument("--root-engine", type=Path)
    parser.add_argument("--root-baseline", type=Path)
    parser.add_argument("--pgo-engine", type=Path)
    parser.add_argument("--pgo-baseline", type=Path)
    args = parser.parse_args()
    require(not args.scratch.resolve().is_relative_to(ARCHIVE), "scratch must be outside the archive")
    args.scratch.mkdir(parents=True, exist_ok=True)
    index = read_json(ARCHIVE / "sha256.json")
    actual = {str(path.relative_to(ARCHIVE)) for path in ARCHIVE.rglob("*") if path.is_file() and path != ARCHIVE / "sha256.json"}
    require(actual == set(index), "artifact inventory differs from index")
    for relative, expected in index.items():
        path = (ARCHIVE / relative).resolve()
        require(path.is_relative_to(ARCHIVE) and sha256(path) == expected, f"artifact hash: {relative}")
    provenance = read_json(ARCHIVE / "provenance.json")
    for relative, expected in {**provenance["source_inputs"], **provenance["measurement_tool_hashes"]}.items():
        require(sha256(ROOT / relative) == expected, f"source identity: {relative}")
    for path, role in [(args.root_engine, "root_candidate"), (args.root_baseline, "root_base"),
                       (args.pgo_engine, "pgo_candidate"), (args.pgo_baseline, "pgo_base")]:
        if path is not None:
            require(sha256(path) == provenance["binary_sha256"][role], f"optional binary identity: {role}")
    with tempfile.TemporaryDirectory(prefix="pgo-root-", dir=args.scratch) as temporary:
        temporary = Path(temporary)
        audit_build(provenance, temporary, args.llvm_profdata)
        audit_matches(provenance, temporary)
    audit_gates(provenance)
    print(f"Audit passed: {len(index)} indexed files, 5 matches, 9120 games; build profiles, source deltas, separate binary pairs and safety evidence verified.")


if __name__ == "__main__":
    main()
