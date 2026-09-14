#!/usr/bin/env python3
"""Audit the evaluation trials, including rejected variants and failed gates."""

import argparse
import gzip
import hashlib
import json
import math
import re
import shutil
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[5]
ARCHIVE = Path(__file__).resolve().parents[1]
sys.dont_write_bytecode = True
sys.path.insert(0, str(ROOT))

from tools import analyze_match, run_sprt, validate_acceptance_contract


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha256(path):
    with path.open("rb") as source:
        digest = hashlib.sha256()
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def read_json(path):
    return json.loads(path.read_text(encoding="utf-8"))


def unpack(source, destination):
    with gzip.open(source, "rb") as packed, destination.open("wb") as output:
        shutil.copyfileobj(packed, output)


def reverse_patch(current, patch):
    """Reconstruct earlier source in memory, verifying every unified-diff hunk."""
    result = dict(current)
    sections = re.split(r"(?=^diff --git )", patch, flags=re.MULTILINE)
    changed = set()
    for section in sections:
        if not section.strip():
            continue
        lines = section.splitlines(keepends=True)
        header = re.fullmatch(r"diff --git a/(\S+) b/(\S+)\n", lines[0])
        require(header is not None and header[1] == header[2], "unsupported patch file header")
        name = header[1]
        require(name in current and name not in changed, f"unexpected patch path: {name}")
        require(f"--- a/{name}\n" in lines and f"+++ b/{name}\n" in lines, "unsupported patch operation")
        changed.add(name)
        source = current[name].splitlines(keepends=True)
        output = []
        cursor = 0
        index = 1
        hunks = 0
        while index < len(lines):
            match = re.match(r"@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@", lines[index])
            index += 1
            if match is None:
                continue
            old_count = int(match[2] or 1)
            new_count = int(match[4] or 1)
            start = int(match[3]) - (1 if new_count else 0)
            require(start >= cursor, "overlapping patch hunks")
            before, after = [], []
            while len(before) < old_count or len(after) < new_count:
                require(index < len(lines), "incomplete patch hunk")
                line = lines[index]
                index += 1
                require(line[:1] in {" ", "+", "-"}, "invalid patch hunk line")
                if line[0] in " -":
                    before.append(line[1:])
                if line[0] in " +":
                    after.append(line[1:])
            require(len(before) == old_count and len(after) == new_count, "patch hunk lengths")
            require(source[start:start + new_count] == after, f"patch context mismatch: {name}")
            output.extend(source[cursor:start])
            output.extend(before)
            cursor = start + new_count
            hunks += 1
        require(hunks > 0, f"no hunks for {name}")
        output.extend(source[cursor:])
        result[name] = "".join(output)
    require(changed, "empty source patch")
    return result


def audit_sources(provenance):
    sources = {name: (ROOT / name).read_text(encoding="utf-8")
               for name in provenance["source_hashes"]["final"]}
    for stage, patch in [("final", None), ("checks", "eval3-accepted-3.patch"),
                         ("pins", "eval3-accepted-2.patch"), ("base", "eval3-accepted-1.patch")]:
        if patch is not None:
            sources = reverse_patch(sources, (ARCHIVE / patch).read_text())
        observed = {name: hashlib.sha256(content.encode()).hexdigest() for name, content in sources.items()}
        require(observed == provenance["source_hashes"][stage], f"source reconstruction: {stage}")
    for name, expected in provenance["tool_hashes"].items():
        require(sha256(ROOT / name) == expected, f"measurement tool changed: {name}")
    for name, expected in provenance["fixture_hashes"].items():
        require(sha256(ROOT / name) == sha256(ARCHIVE / "inputs" / name) == expected,
                f"fixture changed: {name}")
    for name, expected in provenance["test_source_hashes"].items():
        require(sha256(ROOT / name) == expected, f"test harness changed: {name}")
    validate_acceptance_contract.validate_contract(
        ARCHIVE / "inputs/tests/data/dual-channel-null-contract.json", ARCHIVE / "inputs")
    print("Reconstructed all three source patches; original fixture and test-harness identities verified.")


def declared_matches(provenance):
    declarations = {}
    for group, pair in provenance["comparisons"].items():
        protocol = read_json(ARCHIVE / pair["protocol"])
        matches = protocol.get("matches")
        if matches is None:
            require(group == "full-checks", "unexpected single-match protocol")
            matches = [{"name": "eval3-checks-full-screen", **protocol["match"]}]
        for match in matches:
            require(match["name"] not in declarations, "duplicate declared match")
            declarations[match["name"]] = (group, pair, match)
    return declarations


def audit_matches(provenance, temporary):
    books = {}
    for name, info in provenance["books"].items():
        path = ROOT / name
        require(sha256(path) == info["sha256"], f"opening book identity: {name}")
        starts = [" ".join(line.split()[:4]) for line in path.read_text().splitlines()
                  if line.strip() and not line.lstrip().startswith("#")]
        require(len(starts) == info["positions"] == len(set(starts)), f"opening records: {name}")
        books[info["sha256"]] = starts
    declarations = declared_matches(provenance)
    rows = read_json(ARCHIVE / "matches.json")
    require(len(rows) == len(declarations) and {row["stem"] for row in rows} == set(declarations),
            "match inventory differs from the fixed protocols")
    total_games = 0
    for row in rows:
        stem = row["stem"]
        require(re.fullmatch(r"eval3-[a-z-]+", stem) is not None, "unsafe match name")
        group, pair, declared = declarations[stem]
        require(row["comparison"] == group, f"comparison attribution: {stem}")
        pgn = temporary / (stem + ".pgn")
        manifest_path = temporary / (stem + ".manifest.json")
        unpack(ARCHIVE / (stem + ".pgn.gz"), pgn)
        shutil.copyfile(ARCHIVE / manifest_path.name, manifest_path)
        manifest, candidate, baseline, expected_games = analyze_match.load_manifest(manifest_path, pgn)
        summary = read_json(ARCHIVE / (stem + ".sprt.json"))
        execution = manifest["execution"]
        require(summary["status"] == "complete" and not summary["faults"], f"summary faults: {stem}")
        require(execution["return_code"] == 0 and not execution["faults"] and execution["inputs_unchanged"],
                f"execution faults: {stem}")
        require(not run_sprt.arbiter_faults(ARCHIVE / (stem + ".arbiter.json")), f"arbiter faults: {stem}")
        require(summary["inputs"]["manifest_sha256"] == sha256(manifest_path), f"manifest hash: {stem}")
        require(summary["inputs"]["pgn_sha256"] == sha256(pgn), f"PGN hash: {stem}")
        for role in ["candidate", "baseline"]:
            digest = provenance["binary_sha256"][pair[role]]
            require(manifest["inputs"][role]["sha256"] == summary["inputs"][role + "_sha256"] == digest,
                    f"{role} binary pair: {stem}")
            require(manifest["inputs"][role]["aggression"] == 75, f"aggression: {stem}")
        require(summary["profiles"] == {"candidate": 75, "baseline": 75}, f"summary aggression: {stem}")
        require(manifest["inputs"]["runner"]["sha256"] == provenance["binary_sha256"]["eval3-selfplay"], f"runner: {stem}")
        require(manifest["inputs"]["harness"]["sha256"] == provenance["tool_hashes"]["tools/run_sprt.py"], f"harness: {stem}")
        settings = manifest["settings"]
        require(settings["hash_mib"] == 16 and settings["concurrency"] == 20, f"resources: {stem}")
        require(summary["limit"] == settings["limit"] == row["limit"], f"time-limit binding: {stem}")
        limit_name = "movetime_ms" if "movetime_ms" in declared else "time_control"
        require(summary["limit"][limit_name] == declared[limit_name], f"declared time limit: {stem}")
        opening_hash = provenance["books"][declared["book"]]["sha256"]
        require(summary["inputs"]["openings_sha256"] == manifest["inputs"]["openings"]["sha256"] == opening_hash,
                f"opening hash: {stem}")
        games = analyze_match.parse_pgn(pgn)
        require(len(games) == expected_games == declared["games"] == row["games"], f"game count: {stem}")
        for index, game in enumerate(games[::2]):
            require(" ".join(game.fen.split()[:4]) == books[opening_hash][index], f"opening order: {stem}/{index}")
        points = run_sprt.pair_points_from_pgn(pgn, candidate, baseline)
        result = run_sprt.evaluate(points, 0, 10, .05, .05)
        require(result == summary["result"] == row["paired_result"], f"paired statistics: {stem}")
        analysis = analyze_match.summarize(games, manifest, candidate, baseline, pgn, manifest_path)
        require(analysis == read_json(ARCHIVE / (stem + "-analysis.json")), f"WDL and style: {stem}")
        require(analysis["result"] == row["wdl"] and analysis["confidence"]["elo_ci95"] == row["hoeffding_elo_ci95"],
                f"overview statistics: {stem}")
        a, b = analysis["style"]["candidate"], analysis["style"]["baseline"]
        for output, metric in [("forcing_retention_percent", "forcing_moves_per_100_moves"),
                               ("check_retention_percent", "checks_per_100_moves")]:
            require(row[output] == round(100 * a[metric] / b[metric], 6), f"retention: {stem}")
        print(f"{group}: {stem}: {len(games)} games, Elo {result['elo']:+.3f}, {result['sprt']['decision']}")
        total_games += len(games)
        pgn.unlink()
        manifest_path.unlink()
    require(len(rows) == provenance["match_count"] == 9, "total match count")
    require(total_games == provenance["game_count"] == 16192, "total game count")


def audit_efficiency(provenance):
    def geometric(values):
        return math.exp(sum(math.log(value) for value in values) / len(values))

    for stem, pair in provenance["efficiency_pairs"].items():
        report = read_json(ARCHIVE / (stem + "-efficiency.json"))
        for role in ["candidate", "baseline"]:
            require(report["inputs"][role]["sha256"] == provenance["binary_sha256"][pair[role]],
                    f"efficiency binary: {stem}/{role}")
        require(report["inputs"]["suite"]["sha256"] == provenance["fixture_hashes"]["tests/data/search-performance.epd"],
                f"efficiency suite: {stem}")
        rows, metrics = report["positions"], report["metrics"]
        active = [row for row in rows if row["active"] and row["candidate"]["nodes"] > 0 and row["baseline"]["nodes"] > 0]
        throughput = [row for row in rows if row["fixed_nodes"]["candidate"]["nps"] > 0 and row["fixed_nodes"]["baseline"]["nps"] > 0]
        require(len(active) == len(throughput) == len(rows) == 10, f"missing efficiency data: {stem}")
        node_ratio = geometric([row["candidate"]["nodes"] / row["baseline"]["nodes"] for row in active])
        nps_ratio = geometric([row["fixed_nodes"]["candidate"]["nps"] / row["fixed_nodes"]["baseline"]["nps"] for row in throughput])
        depth_gain = sum(row["timed"]["depth_gain"] for row in rows) / len(rows)
        expected = {
            "geometric_candidate_to_baseline_node_ratio": round(node_ratio, 8),
            "geometric_node_reduction_percent": round((1 - node_ratio) * 100, 6),
            "geometric_candidate_to_baseline_nps_ratio": round(nps_ratio, 8),
            "geometric_nps_gain_percent": round((nps_ratio - 1) * 100, 6),
            "mean_completed_depth_gain": round(depth_gain, 6),
            "fixed_depth_candidate_nodes": sum(row["candidate"]["nodes"] for row in rows),
            "fixed_depth_baseline_nodes": sum(row["baseline"]["nodes"] for row in rows),
            "identical_tree_positions": sum(bool(row["identical_tree"]) for row in rows),
            "repeatable_positions": sum(bool(row["repeatable"]) for row in rows),
        }
        require(all(metrics[key] == value for key, value in expected.items()), f"efficiency metrics: {stem}")
        minimum = report["thresholds"]
        passed = (all(row["repeatable"] for row in rows)
                  and all(row[role]["expected_hit"] for row in rows for role in ["candidate", "baseline"])
                  and (not report["gates"]["identical_tree"]["required"] or all(row["identical_tree"] for row in rows))
                  and (1 - node_ratio) * 100 >= minimum["geometric_node_reduction_percent"]["minimum_percent"]
                  and (nps_ratio - 1) * 100 >= minimum["geometric_nps_gain_percent"]["minimum_percent"]
                  and depth_gain >= minimum["mean_completed_depth_gain"]["minimum_plies"])
        require(passed == report["passed"], f"efficiency verdict: {stem}")
        print(f"{stem}: NPS {metrics['geometric_nps_gain_percent']:+.3f}%, depth {depth_gain:+.3f}, diagnostic passed={passed}")


def audit_safety_and_failures(provenance):
    for stem, role in [("eval3-pins", "eval3-pins"), ("eval3-checks-nbr", "eval3-checks-nbr"),
                       ("eval3-races", "eval3-races")]:
        digest = provenance["binary_sha256"][role]
        report = read_json(ARCHIVE / (stem + "-acceptance.json"))
        require(report["inputs"]["engine"]["sha256"] == digest, f"acceptance binary: {stem}")
        failures = [row for row in report["positions"] if not row["passed"]]
        require(len(failures) == 1 and failures[0]["id"] == "standard-opposite-castle-storm", f"unexpected acceptance failure: {stem}")
        failure = failures[0]
        require(failure["gate"] == "personality" and failure["root_loss_cp"] == 0
                and failure["profiles"]["75"]["bestmove"] == "f1e1", f"pawn-storm disposition: {stem}")
        require(all(row["root_loss_passed"] for row in report["positions"]), f"root loss: {stem}")
        require(all(row["passed"] for row in report["positions"] if row["gate"] == "control"), f"safety controls: {stem}")
        require(not report["passed"], "do not relabel the failed all-moves gate as passing")
        require(report["inputs"]["suite"]["sha256"] == provenance["fixture_hashes"]["tests/data/standard-acceptance.epd"], f"acceptance input: {stem}")
        legacy = read_json(ARCHIVE / (stem + "-legacy.json"))
        require(legacy["passed"] and legacy["inputs"]["engine"]["sha256"] == digest, f"legacy gate: {stem}")
        style = read_json(ARCHIVE / (stem + "-style.json"))
        require(style["inputs"]["candidate"]["sha256"] == digest, f"style binary: {stem}")
        require(style["gates"]["candidate_expected_moves"]["passed"] and style["gates"]["controls_preserved"]["passed"], f"endpoint style: {stem}")
        sacrifice = read_json(ARCHIVE / (stem + "-sacrifice.json"))
        require(len(sacrifice["positions"]) == 8 and all(row["status"] == "pass" for row in sacrifice["positions"]), f"sacrifice gate: {stem}")
    base = read_json(ARCHIVE / "eval3-base-acceptance.json")
    require(base["passed"] and base["inputs"]["engine"]["sha256"] == provenance["binary_sha256"]["eval3-base"], "original standard acceptance")
    rejected = read_json(ARCHIVE / "eval3-checks-full-legacy.json")
    require(not rejected["passed"] and any(row["id"] == "contract-avoid-equal-queen-trade" and not row["passed"] for row in rejected["positions"]), "rejected queen-trade regression")
    for filename, needle in [("eval3-pins-red-tests.log", "2 tests run: 0 passed, 2 failed"),
                             ("eval3-checks-red-tests.log", "2/3 tests run: 0 passed, 2 failed"),
                             ("eval3-checks-full-failed-tests.log", "null-in-check changed best move"),
                             ("eval3-checks-filtered-failed-tests.log", "avoid-equal-queen-trade")]:
        require(needle in (ARCHIVE / filename).read_text(), f"missing original failure: {filename}")
    for stem, counts in [("pins", (297, 314)), ("checks-nbr", (302, 320)), ("races", (307, 326))]:
        log = (ARCHIVE / f"eval3-{stem}-tests.log").read_text()
        for count in counts:
            require(f"{count} tests run: {count} passed" in log, f"Rust test count: {stem}/{count}")
    total = provenance["python_tests"]
    require(f"{total} passed" in (ARCHIVE / "eval3-final-python.log").read_text(), "Python validation count")
    print("Original failures remain explicit; no root-loss cap, fixture or personality test was relaxed.")


def audit_calibration(provenance):
    protocol = read_json(ARCHIVE / "eval3-safe-refit-protocol.json")
    training = ROOT / "docs/tuning/data/evaluation-refit-pilot/training.filtered.txt.gz"
    require(sha256(training) == protocol["training_archive_sha256"], "calibration training archive")
    with gzip.open(training, "rb") as source:
        content = source.read()
    require(hashlib.sha256(content).hexdigest() == protocol["training_sha256"], "calibration training content")
    require(len(content.splitlines()) == 140541, "calibration training records")
    pattern = r"const SAFE_CHECK_BY_PIECE: \[ScorePair; 4\] = \[(.*?)\];"
    fitted = re.search(pattern, (ARCHIVE / "eval3-safe-calibrated.fit.rs").read_text(), re.S)
    current = re.search(pattern, (ROOT / "src/engine/evaluation/weights.rs").read_text(), re.S)
    require(fitted is not None and current is not None, "safe-check weight declarations")
    pairs = lambda match: re.findall(r"ScorePair::new\((-?\d+), (-?\d+)\)", match[1])
    require(pairs(fitted) == pairs(current) == [("23", "-1"), ("15", "15"), ("25", "9"), ("45", "15")], "calibration emitted different weights")
    require("607 of 611 features held" in (ARCHIVE / "eval3-safe-calibrated.fit.log").read_text(), "calibration held blocks")
    print("The four-weight calibration emitted unchanged values; no fitted gain is claimed.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scratch", type=Path, required=True)
    parser.add_argument("--binary", action="append", default=[], metavar="ROLE=PATH", help="optionally verify an immutable executable")
    args = parser.parse_args()
    require(not args.scratch.resolve().is_relative_to(ARCHIVE), "scratch must be outside the archive")
    args.scratch.mkdir(parents=True, exist_ok=True)
    index = read_json(ARCHIVE / "sha256.json")
    actual = {str(path.relative_to(ARCHIVE)) for path in ARCHIVE.rglob("*")
              if path.is_file() and path != ARCHIVE / "sha256.json"}
    require(actual == set(index), "artifact inventory differs from the index")
    for name, digest in index.items():
        path = (ARCHIVE / name).resolve()
        require(path.is_relative_to(ARCHIVE) and sha256(path) == digest, f"artifact identity: {name}")
    provenance = read_json(ARCHIVE / "provenance.json")
    for specification in args.binary:
        role, separator, path = specification.partition("=")
        require(separator and role in provenance["binary_sha256"], f"unknown binary specification: {role}")
        require(sha256(Path(path)) == provenance["binary_sha256"][role], f"binary identity: {role}")
    audit_sources(provenance)
    with tempfile.TemporaryDirectory(prefix="eval-trials-", dir=args.scratch) as temporary:
        audit_matches(provenance, Path(temporary))
    audit_efficiency(provenance)
    audit_safety_and_failures(provenance)
    audit_calibration(provenance)
    print(f"Audit passed: {len(index)} indexed files, 9 matches, 16192 games; source stages, separate binary pairs, failures and uncertainty verified.")


if __name__ == "__main__":
    main()
