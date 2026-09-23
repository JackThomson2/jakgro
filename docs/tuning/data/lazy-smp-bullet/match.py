#!/usr/bin/env python3
"""Run a paired timed match with an explicit grace and preserved provenance."""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path.cwd()))
from tools import run_sprt


def main():
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--time-grace-ms", type=int, default=10)
    extra, rest = parser.parse_known_args()
    if extra.time_grace_ms < 0:
        parser.error("time grace must be nonnegative")
    args = run_sprt.parse_arguments(rest)
    if args.time_control is None and args.movetime_ms is None:
        parser.error("this launcher requires a timed limit")
    args.engine = run_sprt.resolve_executable(args.engine)
    args.baseline_engine = run_sprt.resolve_executable(args.baseline_engine or args.engine)
    args.runner = run_sprt.resolve_executable(args.runner)
    args.openings = args.openings.resolve(strict=True)
    for name in ("candidate_eval_file", "baseline_eval_file"):
        if getattr(args, name) is not None:
            setattr(args, name, getattr(args, name).resolve(strict=True))
    args.pgn = args.pgn.resolve()
    for field, suffix in (("manifest", ".manifest.json"), ("summary_json", ".sprt.json"), ("results_json", ".arbiter.json")):
        value = getattr(args, field)
        setattr(args, field, value.resolve() if value is not None else args.pgn.with_suffix(suffix))
    for path in (args.pgn, args.manifest, args.summary_json, args.results_json):
        if path.exists():
            parser.error(f"refusing to overwrite existing evidence: {path}")
        path.parent.mkdir(parents=True, exist_ok=True)
    candidate, baseline = run_sprt.engine_names(args)
    command = run_sprt.build_command(args, candidate, baseline)
    command += ["--time-grace-ms", str(extra.time_grace_ms)]
    opening_count = sum(1 for line in args.openings.read_text().splitlines() if line.strip() and not line.strip().startswith("#"))
    manifest = run_sprt.build_manifest(args, command, candidate, baseline, opening_count)
    manifest["settings"]["time_grace_ms"] = extra.time_grace_ms
    manifest["inputs"]["launcher"] = {"path": str(Path(__file__).resolve()), "sha256": run_sprt.sha256_file(Path(__file__))}
    manifest["host"]["affinity"] = sorted(__import__("os").sched_getaffinity(0))
    manifest["execution"] = {"status": "running"}
    run_sprt.write_json(args.manifest, manifest)
    print(json.dumps({"command": command, "manifest": str(args.manifest)}), flush=True)
    started = datetime.now(timezone.utc)
    completed = subprocess.run(command, check=False)
    finished = datetime.now(timezone.utc)
    run_sprt.record_execution(manifest, args, started, finished, completed.returncode, None)
    run_sprt.write_json(args.manifest, manifest)
    points = run_sprt.pair_points_from_pgn(args.pgn, candidate, baseline)
    evaluation = run_sprt.evaluate(points, args.elo0, args.elo1, args.alpha, args.beta)
    summary = {
        "engines": {"candidate": candidate, "baseline": baseline},
        "status": manifest["execution"]["status"],
        "faults": manifest["execution"]["faults"],
        "settings": manifest["settings"],
        "inputs": {"pgn_sha256": run_sprt.sha256_file(args.pgn),
                   "manifest_sha256": run_sprt.sha256_file(args.manifest),
                   "candidate_sha256": manifest["inputs"]["candidate"]["sha256"],
                   "baseline_sha256": manifest["inputs"]["baseline"]["sha256"]},
        "result": evaluation,
    }
    run_sprt.write_json(args.summary_json, summary)
    print(json.dumps(summary, indent=2), flush=True)
    return 0 if summary["status"] == "complete" else 3


if __name__ == "__main__":
    raise SystemExit(main())
