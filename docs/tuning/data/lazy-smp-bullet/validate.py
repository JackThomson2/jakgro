#!/usr/bin/env python3
"""Audit archived SMP observations and match results without running an engine."""
from __future__ import annotations

from datetime import datetime
import gzip
import hashlib
import importlib.util
from itertools import product
import json
import math
from pathlib import Path
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[4]
ARCHIVE = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT))
from tools import analyze_match, run_sprt
from tools.measure_style import parse_suite

VARIANTS = ("baseline", "helpers", "tt", "diversified")
MATCHES = (
    ("helpers-screen", "helpers", "baseline", 128, "1+0.01", 1),
    ("tt-screen", "tt", "helpers", 128, "1+0.01", 1),
    ("diversified-screen", "diversified", "tt", 256, "1+0.01", 1),
    ("bullet", "diversified", "baseline", 24, "60+0.1", 2),
)


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(content):
    return hashlib.sha256(content).hexdigest()


def read_json(path):
    return json.loads(path.read_text(encoding="utf-8"))


def fixed_grid(data, fixtures):
    expected = set(product(VARIANTS, fixtures, ("nodes", "depth"), range(2)))
    rows = data["fixed"]
    indexed = {(r["variant"], r["fixture"], r["mode"], r["repeat"]): r["signature"] for r in rows}
    require(len(rows) == 160 and len(indexed) == len(rows) and set(indexed) == expected,
            "fixed observation grid is incomplete or duplicated; skipped checks are not equality")
    for (variant, fixture, mode, repeat), signature in indexed.items():
        require(signature == indexed[("baseline", fixture, mode, 0)],
                f"fixed signature differs: {variant}, {fixture}, {mode}, repeat {repeat}")
    return len(rows)


def validate_timing():
    raw = gzip.decompress((ARCHIVE / "timing.json.gz").read_bytes())
    data = json.loads(raw)
    summary = read_json(ARCHIVE / "timing-summary.json")
    require(digest(raw) == summary["raw_sha256"], "timing raw hash differs")
    require(digest((ARCHIVE / "measure.py").read_bytes()) == data["inputs"]["script_sha256"],
            "as-run timing script hash differs")
    require(digest((ARCHIVE / "performance.epd").read_bytes()) == data["inputs"]["suite_sha256"],
            "timing suite hash differs")
    fixtures = tuple(f.identifier for f in parse_suite(ARCHIVE / "performance.epd"))
    require(len(fixtures) == 10, "timing suite must contain ten fixtures")
    fixed_count = fixed_grid(data, fixtures)
    rows = data["timed"]
    expected = set(product(VARIANTS, fixtures, (1, 8), (50, 250), range(12)))
    actual = {(r["variant"], r["fixture"], r["threads"], r["movetime_ms"], r["sample"]) for r in rows}
    require(len(rows) == 1920 and len(actual) == len(rows) and actual == expected,
            "timed observation grid is incomplete or duplicated")
    for row in rows:
        require(row["reported_nps"] > 0 and row["nodes"] > 0, "nonpositive timing observation")
        require(all(math.isfinite(row[key]) for key in ("wall_ms", "first_info_ms")),
                "nonfinite wall timing")
        require(0 <= row["first_info_ms"] <= row["wall_ms"], "invalid first-info/wall ordering")
    require(summary["observed_fixed_count"] == fixed_count and summary["observed_timed_count"] == len(rows),
            "summary observation counts differ")
    for key in data:
        if key not in ("fixed", "timed"):
            require(summary[key] == data[key], f"timing summary field differs: {key}")
    specification = importlib.util.spec_from_file_location("archived_smp_measure", ARCHIVE / "measure.py")
    module = importlib.util.module_from_spec(specification)
    specification.loader.exec_module(module)
    recomputed = module.summaries(rows, list(VARIANTS), [50, 250], [1, 8])
    require(recomputed == data["summaries"], "timing statistics do not reproduce")
    for comparison in data["fixed_comparisons"]:
        require(not comparison["differences"], "recorded fixed comparison reports a difference")
    return data, {"fixed_observations": fixed_count, "timed_observations": len(rows)}


def validate_match(case, binaries, network_hash, screen_book):
    name, candidate_label, baseline_label, games_expected, clock, concurrency = case
    manifest_path = ARCHIVE / f"{name}.manifest.json"
    summary = read_json(ARCHIVE / f"{name}.sprt.json")
    arbiter = read_json(ARCHIVE / f"{name}.arbiter.json")
    raw = gzip.decompress((ARCHIVE / f"{name}.pgn.gz").read_bytes())
    with tempfile.TemporaryDirectory(prefix="smp-audit-") as directory:
        pgn = Path(directory) / f"{name}.pgn"
        pgn.write_bytes(raw)
        manifest, candidate, baseline, count = analyze_match.load_manifest(manifest_path, pgn)
        games = analyze_match.parse_pgn(pgn)
        analysis = analyze_match.summarize(games, manifest, candidate, baseline, pgn, manifest_path)
        require(analysis == read_json(ARCHIVE / f"{name}.analysis.json"), f"{name}: analysis differs")
        points = run_sprt.pair_points_from_pgn(pgn, candidate, baseline)
    settings = manifest["settings"]
    require(count == games_expected and len(games) == games_expected, f"{name}: game count differs")
    require(settings["threads"] == 8 and settings["hash_mib"] == 16, f"{name}: engine settings differ")
    require(settings["concurrency"] == concurrency and settings["time_grace_ms"] == 10,
            f"{name}: concurrency or grace differs")
    require(settings["limit"]["mode"] == "fixed-time" and settings["time_control"] == clock,
            f"{name}: clock differs")
    require(manifest["execution"]["inputs_unchanged"] is True, f"{name}: inputs changed")
    require(not manifest["execution"]["faults"] and not arbiter["faults"] and not summary["faults"],
            f"{name}: recorded faults")
    require(summary["status"] == "complete" and summary["settings"] == settings, f"{name}: summary settings differ")
    require(summary["inputs"]["pgn_sha256"] == digest(raw), f"{name}: summary PGN hash differs")
    require(summary["inputs"]["manifest_sha256"] == digest(manifest_path.read_bytes()),
            f"{name}: summary manifest hash differs")
    command = manifest["command"]
    for option, value in (("--threads", "8"), ("--hash", "16"), ("--concurrency", str(concurrency)),
                          ("--time-control", clock), ("--time-grace-ms", "10")):
        require(command.count(option) == 1 and command[command.index(option) + 1] == value,
                f"{name}: executed command differs for {option}")
    for role, label in (("candidate", candidate_label), ("baseline", baseline_label)):
        entry = manifest["inputs"][role]
        require(entry["sha256"] == binaries[label] == summary["inputs"][f"{role}_sha256"],
                f"{name}: {role} binary hash differs")
        require(entry["aggression"] == 75 and entry["eval_file"]["sha256"] == network_hash,
                f"{name}: {role} evaluation differs")
    require(manifest["inputs"]["runner"]["sha256"] == binaries["selfplay"], f"{name}: runner hash differs")
    require(manifest["inputs"]["launcher"]["sha256"] == digest((ARCHIVE / "match.py").read_bytes()),
            f"{name}: launcher hash differs")
    book = (ARCHIVE / "bullet-openings.epd").read_bytes() if name == "bullet" else screen_book
    require(manifest["inputs"]["openings"]["sha256"] == digest(book), f"{name}: opening hash differs")
    sprt = settings["sprt"]
    recomputed = run_sprt.evaluate(points, sprt["elo0"], sprt["elo1"], sprt["alpha"], sprt["beta"])
    require(recomputed == summary["result"], f"{name}: paired statistics differ")
    return manifest, {"match": name, "games": count, "faults": 0, "result": analysis["result"],
                      "sprt_decision": recomputed["sprt"]["decision"]}


def main():
    timing, counts = validate_timing()
    provenance = read_json(ARCHIVE / "provenance.json")
    binaries = {entry["label"]: entry["sha256"] for entry in provenance["binaries"]}
    for label in VARIANTS:
        require(timing["variants"][label]["sha256"] == binaries[label], f"{label}: timing binary hash differs")
    screen_book = gzip.decompress((ARCHIVE / "screen-openings.epd.gz").read_bytes())
    openings = [line for line in screen_book.decode().splitlines() if line.strip() and not line.startswith("#")]
    bullet = (ARCHIVE / "bullet-openings.epd").read_text().splitlines()
    require(bullet == openings[256:268], "bullet openings are not the recorded held-out slice")
    require(not ({line.split(';')[0].strip() for line in bullet} &
                 {line.split(';')[0].strip() for line in openings[:128]}), "bullet/screen openings overlap")
    manifests, matches = [], []
    for case in MATCHES:
        manifest, report = validate_match(case, binaries, timing["inputs"]["network_sha256"], screen_book)
        manifests.append(manifest)
        matches.append(report)
    for current in manifests:
        at = datetime.fromisoformat(current["execution"]["started_utc"])
        active = [m for m in manifests if datetime.fromisoformat(m["execution"]["started_utc"]) <= at <
                  datetime.fromisoformat(m["execution"]["finished_utc"])]
        reserved = sum(2 * m["settings"]["threads"] * m["settings"]["concurrency"] for m in active)
        require(reserved <= provenance["physical_cores"], "recorded matches oversubscribed the core budget")
        for index, left in enumerate(active):
            for right in active[index + 1:]:
                require(set(left["host"]["affinity"]).isdisjoint(right["host"]["affinity"]),
                        "concurrent matches shared an affinity set")
    print(json.dumps({"status": "verified", **counts, "matches": matches,
                      "not_replayed": ["engine binaries", "matches", "timing samples"]}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
