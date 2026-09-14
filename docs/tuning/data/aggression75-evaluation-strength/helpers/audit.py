#!/usr/bin/env python3
"""Audit retained trial evidence without building an engine or playing games."""

from __future__ import annotations

import argparse
import csv
import gzip
import importlib.util
import json
import math
import sys
import tempfile
from pathlib import Path

sys.dont_write_bytecode = True
from archive_support import ARCHIVE, ROOT, digest, read_json, require, source_digests, stages


def frozen_tool(name: str):
    path = ARCHIVE / "inputs/tools" / (name + ".py")
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


analyze = frozen_tool("analyze_match")
sprt = frozen_tool("run_sprt")
style = frozen_tool("measure_style")
acceptance = frozen_tool("measure_acceptance")
efficiency = frozen_tool("measure_search_efficiency")


def audit_inputs(provenance: dict) -> int:
    index = read_json(ARCHIVE / "sha256.json")
    actual = {str(path.relative_to(ARCHIVE)) for path in ARCHIVE.rglob("*") if path.is_file()}
    require(actual == set(index) | {"sha256.json"}, "archive inventory differs from index")
    for name, expected in index.items():
        require(digest((ARCHIVE / name).read_bytes()) == expected, f"archive digest: {name}")
    for name, expected in provenance["frozen_inputs"].items():
        require(digest((ARCHIVE / "inputs" / name).read_bytes()) == expected, f"frozen input: {name}")
        require(digest((ROOT / name).read_bytes()) == expected, f"workspace input changed: {name}")
    for channel, record in provenance["corpora"].items():
        payload = (ROOT / record["path"]).read_bytes()
        require(digest(payload) == record["sha256"], f"corpus digest: {channel}")
        unpacked = gzip.decompress(payload)
        require(digest(unpacked) == record["uncompressed_sha256"], f"corpus payload: {channel}")
        require(len(unpacked.splitlines()) == record["rows"], f"corpus rows: {channel}")
    reconstructed = stages()
    require(set(reconstructed) == set(provenance["source_stages"]), "source stage inventory")
    for name, source in reconstructed.items():
        require(source_digests(source) == provenance["source_stages"][name], f"source stage: {name}")
    return len(index)


def audit_matches(provenance: dict, scratch: Path) -> list[dict]:
    book_path = "docs/tuning/data/evaluation-refit-pilot/development.epd"
    book = ARCHIVE / "inputs" / book_path
    openings = [" ".join(line.split()[:4]) for line in book.read_text().splitlines()
                if line.strip() and not line.lstrip().startswith("#")]
    require(len(openings) == len(set(openings)) == 512, "development book identity")
    results = []
    for name in provenance["matches"]:
        original = ARCHIVE / "runs" / name
        pgn = scratch / (name + ".pgn")
        pgn.write_bytes(gzip.decompress(original.with_suffix(".pgn.gz").read_bytes()))
        manifest_path = original.with_suffix(".manifest.json")
        manifest, candidate, baseline, count = analyze.load_manifest(manifest_path, pgn)
        summary = read_json(original.with_suffix(".sprt.json"))
        arbiter = read_json(original.with_suffix(".arbiter.json"))
        execution = read_json(original.with_suffix(".execution.json"))
        protocol = read_json(original.with_suffix(".protocol.json"))
        require(count == 1024 and manifest["settings"]["concurrency"] == 16, f"match cap: {name}")
        require(manifest["settings"]["hash_mib"] == 16, f"match hash size: {name}")
        require(manifest["settings"]["limit"]["mode"] == "fixed-movetime"
                and manifest["settings"]["limit"]["movetime_ms"] == 50, f"match time: {name}")
        require(summary["status"] == "complete" and not summary["faults"], f"match status: {name}")
        require(execution["exit_status"] == 0 and execution["inputs_unchanged"], f"execution: {name}")
        require(execution["hashes_before"] == execution["hashes_after"] == protocol["hashes_before"],
                f"input rehash: {name}")
        require(execution["command"] == protocol["command"], f"declared command: {name}")
        for role, binary in (("candidate", candidate), ("baseline", "refit-base"), ("runner", "selfplay")):
            recorded = manifest["inputs"][role]
            expected = provenance["binaries"][binary]["sha256"]
            require(recorded["sha256"] == expected, f"binary attribution: {name}/{role}")
            require(execution["hashes_before"][recorded["path"]] == expected, f"executed binary: {name}/{role}")
            if role != "runner":
                require(recorded["aggression"] == 75, f"aggression: {name}/{role}")
        require(manifest["inputs"]["openings"]["sha256"] == digest(book.read_bytes()), f"book hash: {name}")
        require(manifest["inputs"]["harness"]["sha256"] == provenance["frozen_inputs"]["tools/run_sprt.py"],
                f"harness identity: {name}")
        games = analyze.parse_pgn(pgn)
        require(len(games) == 1024, f"PGN count: {name}")
        for index, opening in enumerate(openings):
            pair = games[2 * index:2 * index + 2]
            require(all(" ".join(game.fen.split()[:4]) == opening for game in pair),
                    f"opening order: {name}/{index}")
        points = sprt.pair_points_from_pgn(pgn, candidate, baseline)
        computed = sprt.evaluate(points, 0, 10, 0.05, 0.05)
        require(computed == summary["result"], f"paired statistics: {name}")
        require(points == arbiter["pair_points"] and arbiter["games"] == 1024
                and arbiter["pairs"] == 512 and not arbiter["faults"], f"arbiter agreement: {name}")
        analysis = analyze.summarize(games, manifest, candidate, baseline, pgn, manifest_path)
        require(analysis == read_json(original.with_suffix(".analysis.json")), f"analysis: {name}")
        require(all(analysis["result"][key] == arbiter[key] for key in ("wins", "draws", "losses")),
                f"W/D/L: {name}")
        require(analysis["terminations"] == arbiter["terminations"], f"terminations: {name}")
        require(summary["inputs"]["pgn_sha256"] == digest(pgn.read_bytes())
                and summary["inputs"]["manifest_sha256"] == digest(manifest_path.read_bytes()),
                f"summary binding: {name}")
        require(computed["elo_ci95"][0] < 0 < computed["elo_ci95"][1]
                and computed["sprt"]["decision"] == "continue", f"inconclusive verdict: {name}")
        rates = analysis["style"]
        forcing = 100 * rates["candidate"]["forcing_moves_per_100_moves"] / rates["baseline"]["forcing_moves_per_100_moves"]
        checks = 100 * rates["candidate"]["checks_per_100_moves"] / rates["baseline"]["checks_per_100_moves"]
        results.append({"name": name, "games": count, "elo": computed["elo"],
                        "elo_ci95": computed["elo_ci95"], "forcing_retention_percent": forcing,
                        "check_retention_percent": checks})
    return results


def weights(path: Path) -> list[tuple[int, int]]:
    with path.open() as stream:
        rows = list(csv.DictReader(stream, delimiter="\t"))
    require([int(row["index"]) for row in rows] == list(range(611)), f"weight indices: {path}")
    return [(int(row["mg"]), int(row["eg"])) for row in rows]


def audit_fits(provenance: dict) -> None:
    data = ARCHIVE / "data"
    base = weights(data / "base.tsv")
    require(base == weights(data / "source.tsv"), "base source/compiled weights")
    with (data / "layout.tsv").open() as stream:
        blocks = list(csv.DictReader(stream, delimiter="\t"))
    for name in provenance["fits"]:
        record = read_json(data / (name + ".record.json"))
        fitted = weights(data / (name + ".tsv"))
        changed = [index for index, (before, after) in enumerate(zip(base, fitted)) if before != after]
        require(record["exit_status"] == 0 and len(changed) == record["changed_pairs"], f"fit count: {name}")
        require(max(abs(a - b) for before, after in zip(base, fitted) for a, b in zip(before, after))
                == record["max_coordinate_delta"], f"fit displacement: {name}")
        require(digest((data / (name + ".tsv")).read_bytes()) == record["emitted_sha256"], f"fit hash: {name}")
        family = name.split("-")[1]
        held = set((data / (family + ".hold")).read_text().split(","))
        for block in blocks:
            lo, length = int(block["offset"]), int(block["len"])
            if block["name"] in held:
                require(base[lo:lo + length] == fitted[lo:lo + length], f"held block: {name}/{block['name']}")
        for channel in ("training", "development"):
            loss = read_json(data / (name + "." + channel + ".json"))
            require(loss == record[channel] and loss["positions"] == provenance["corpora"][channel]["rows"],
                    f"fit loss report: {name}/{channel}")
            require(math.isclose(loss["k"], float((data / "calibration.k").read_text()), rel_tol=0, abs_tol=1e-15),
                    f"fixed calibration: {name}")
        fitter = "safe-tune" if name.startswith("safe-") else "refit-tune-base"
        require(record["fitter_sha256"] == provenance["binaries"][fitter]["sha256"], f"fitter identity: {name}")
        if name in {"raw-mobility-1", "safe-mobility-1"}:
            require(fitted == weights(data / (name + ".applied.tsv")), f"installed fit: {name}")
    for phase in ("training", "development"):
        require(read_json(data / ("base." + phase + ".json"))["positions"] == provenance["corpora"][phase]["rows"],
                f"base loss sample count: {phase}")


def audit_gates_and_speed(provenance: dict) -> None:
    data = ARCHIVE / "data"
    for name in ("base-acceptance", "base-legacy-acceptance", "safe-mobility-1.acceptance",
                 "raw-mobility-1.acceptance", "memo-8192.acceptance", "memo-tagged.acceptance"):
        report = read_json(data / (name + ".json"))
        for row in report["positions"]:
            for observation in row["profiles"].values():
                require(observation["expected_hit"] == (observation["bestmove"] in observation["expected"]),
                        f"expected move flag: {name}/{row['id']}")
            expected = all(observation["expected_hit"] for observation in row["profiles"].values())
            loss = max(0, acceptance.score_to_cp(row["objective"]["score"]) -
                       acceptance.score_to_cp(row["selected_under_objective"]["score"]))
            require(row["root_loss_cp"] == loss and row["root_loss_passed"] == (loss <= row["maximum_loss_cp"]),
                    f"loss gate: {name}/{row['id']}")
            require(row["expected_moves_passed"] == expected
                    and row["passed"] == (expected and row["root_loss_passed"]), f"position verdict: {name}")
        computed = acceptance.summarize(report["positions"], data / "base.tsv", data / "layout.tsv")
        require(computed["gates"] == report["gates"] and computed["passed"] == report["passed"], f"acceptance: {name}")
    for name in ("base-endpoints", "base-sacrifice", "base-style", "safe-mobility-1.endpoints",
                 "safe-mobility-1.sacrifice", "raw-mobility-1.endpoints", "raw-mobility-1.sacrifice",
                 "memo-8192.endpoints", "memo-8192.sacrifice", "memo-tagged.endpoints", "memo-tagged.sacrifice"):
        report = read_json(data / (name + ".json"))
        computed = style.summarize(report["positions"])
        require(computed["categories"] == report["categories"], f"style summary: {name}")
    for name in ("memo-8192", "memo-tagged"):
        report = read_json(data / (name + ".efficiency.json"))
        rows = report["positions"]
        for row in rows:
            signature = lambda value: tuple(value[key] for key in ("bestmove", "score", "depth", "nodes"))
            require(signature(row["candidate"]) == signature(row["baseline"]), f"fixed tree: {name}/{row['id']}")
            for role in ("candidate", "baseline"):
                require(signature(row[role]) == signature(row["fixed_depth_repeats"][role]), f"repeatability: {name}")
        computed = efficiency.summarize(rows, data / "base.tsv", data / "base.tsv", data / "layout.tsv",
                                        75, 8, 0, samples=7, move_time_ms=500, require_identical_tree=True)
        for key in ("metrics", "gates", "thresholds", "passed"):
            require(computed[key] == report[key], f"efficiency {key}: {name}")
        require(report["inputs"]["candidate"]["sha256"] == provenance["binaries"][name]["sha256"], f"speed binary: {name}")
    require(read_json(data / "memo-tagged.gates.json") == {"endpoints": 0, "sacrifice": 0, "acceptance": 0},
            "retained memo gates")
    require(not read_json(data / "raw-mobility-1.acceptance.json")["passed"]
            and not read_json(data / "safe-mobility-1.acceptance.json")["passed"], "rejected gate results")


def audit_validation() -> None:
    validation = read_json(ARCHIVE / "validation.json")
    for command in validation["commands"]:
        log = (ARCHIVE / command["log"]).read_text()
        require(command["exit_status"] == 0, "accepted validation status")
        if "tests" in command:
            count = command["tests"]
            require(f"{count} tests run: {count} passed, 0 skipped" in log, f"test summary: {command['log']}")
    failure = (ARCHIVE / validation["initial_trace_test"]["log"]).read_text()
    require("77 passed, 1 failed" in failure and "left: 172" in failure and "right: 171" in failure,
            "original trace mismatch evidence")
    require("mismatches=16" in (ARCHIVE / validation["baseline_wrong_profile_invocation"]["log"]).read_text(),
            "original profile invocation evidence")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scratch", type=Path, required=True)
    parser.add_argument("--binary", action="append", default=[], metavar="ROLE=PATH")
    args = parser.parse_args()
    scratch = args.scratch.resolve()
    require(scratch != ROOT and ROOT not in scratch.parents, "scratch must be outside the repository")
    scratch.mkdir(parents=True, exist_ok=True)
    provenance = read_json(ARCHIVE / "provenance.json")
    indexed = audit_inputs(provenance)
    for binding in args.binary:
        role, path = binding.split("=", 1)
        require(role in provenance["binaries"], f"unknown binary role: {role}")
        require(digest(Path(path).read_bytes()) == provenance["binaries"][role]["sha256"], f"binary hash: {role}")
    with tempfile.TemporaryDirectory(prefix="eval-audit-", dir=scratch) as temporary:
        matches = audit_matches(provenance, Path(temporary))
    audit_fits(provenance)
    audit_gates_and_speed(provenance)
    audit_validation()
    result = {"passed": True, "indexed_files": indexed, "fits": 8,
              "games": sum(match["games"] for match in matches), "matches": matches,
              "verdict": "No demonstrated Elo gain; original baseline remains the strength recommendation.",
              "limits": "Audit recomputes game and diagnostic statistics, not legal game replay, timed searches or fitted-model MSE."}
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, KeyError) as error:
        print(f"audit: {error}", file=sys.stderr)
        raise SystemExit(1)
