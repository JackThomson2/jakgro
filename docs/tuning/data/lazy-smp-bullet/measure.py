#!/usr/bin/env python3
"""Compare saved engine binaries with paired timed UCI samples and wall latency."""
from __future__ import annotations

import argparse
from contextlib import ExitStack
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import random
import statistics
import sys
import time

sys.path.insert(0, str(Path.cwd()))
from tools.measure_style import UciEngine, parse_suite


class MeasuredEngine(UciEngine):
    def __init__(self, executable: Path, hash_mib: int):
        self.go_started = None
        self.first_info_ms = None
        self.wall_ms = None
        self.last_info = None
        self.options = []
        super().__init__(executable, 30.0)
        if not any(line.startswith("option name Threads type spin ") for line in self.options):
            self.close()
            raise RuntimeError(f"{executable}: Threads is not advertised")
        self.send(f"setoption name Hash value {hash_mib}")
        self.send("setoption name Use NNUE value true")
        self.send("isready")
        output = self.read_until(lambda line: line == "readyok")
        if any("rejected" in line or "requires" in line for line in output):
            self.close()
            raise RuntimeError(f"{executable}: configuration rejected: {output}")

    def send(self, command: str):
        if command.startswith("go "):
            self.go_started = time.perf_counter()
            self.first_info_ms = None
            self.wall_ms = None
            self.last_info = None
        super().send(command)

    def read_until(self, predicate):
        def observe(line):
            if line.startswith("option name "):
                self.options.append(line)
            if self.go_started is not None:
                elapsed = (time.perf_counter() - self.go_started) * 1000.0
                if line.startswith("info depth "):
                    if self.first_info_ms is None:
                        self.first_info_ms = elapsed
                    self.last_info = line
                if line.startswith("bestmove "):
                    self.wall_ms = elapsed
                    self.go_started = None
            return predicate(line)
        return super().read_until(observe)

    def sample(self, fixture, aggression, **limits):
        observation = self.measure(fixture, aggression, **limits)
        return {
            "bestmove": observation.bestmove,
            "score": observation.score,
            "depth": observation.depth,
            "nodes": observation.nodes,
            "reported_ms": observation.elapsed_ms,
            "reported_nps": observation.nps,
            "wall_ms": round(self.wall_ms, 6),
            "first_info_ms": round(self.first_info_ms, 6),
            "pv": self.last_info.split(" pv ", 1)[1].split(),
        }


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def signature(row):
    return {key: row[key] for key in ("bestmove", "score", "depth", "nodes", "pv")}


def percentile(values, p):
    values = sorted(values)
    at = (len(values) - 1) * p
    lower = math.floor(at)
    upper = math.ceil(at)
    return values[lower] + (values[upper] - values[lower]) * (at - lower)


def summaries(rows, names, times, thread_counts):
    result = []
    for threads in thread_counts:
        for milliseconds in times:
            subset = [r for r in rows if r["threads"] == threads and r["movetime_ms"] == milliseconds]
            fixtures = sorted({r["fixture"] for r in subset})
            medians = {}
            for name in names:
                medians[name] = {}
                for fixture in fixtures:
                    group = [r for r in subset if r["variant"] == name and r["fixture"] == fixture]
                    medians[name][fixture] = {
                        key: statistics.median(r[key] for r in group)
                        for key in ("reported_nps", "depth", "nodes", "wall_ms", "first_info_ms")
                    }
            comparisons = []
            pairs = list(zip(names[1:], names[:-1]))
            if len(names) > 2:
                pairs.append((names[-1], names[0]))
            for candidate, baseline in pairs:
                logs = [math.log(medians[candidate][f]["reported_nps"] / medians[baseline][f]["reported_nps"]) for f in fixtures]
                rng = random.Random(1729)
                draws = [math.expm1(statistics.mean(rng.choices(logs, k=len(logs)))) * 100 for _ in range(5000)]
                comparisons.append({
                    "candidate": candidate,
                    "baseline": baseline,
                    "geometric_reported_nps_gain_percent": round(math.expm1(statistics.mean(logs)) * 100, 4),
                    "fixture_bootstrap_ci95_percent": [round(percentile(draws, p), 4) for p in (0.025, 0.975)],
                    "mean_median_depth_gain": round(statistics.mean(medians[candidate][f]["depth"] - medians[baseline][f]["depth"] for f in fixtures), 4),
                })
            latency = {}
            for name in names:
                group = [r for r in subset if r["variant"] == name]
                latency[name] = {
                    key: {"p50": round(percentile([r[key] for r in group], 0.5), 4),
                          "p95": round(percentile([r[key] for r in group], 0.95), 4),
                          "max": round(max(r[key] for r in group), 4)}
                    for key in ("wall_ms", "first_info_ms")
                }
            result.append({"threads": threads, "movetime_ms": milliseconds,
                           "comparisons": comparisons, "latency": latency, "fixture_medians": medians})
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--variant", action="append", required=True, help="LABEL=EXECUTABLE, in cumulative patch order")
    parser.add_argument("--suite", type=Path, default=Path("tests/data/search-performance.epd"))
    parser.add_argument("--samples", type=int, default=12)
    parser.add_argument("--times-ms", type=int, nargs="+", default=[50, 250])
    parser.add_argument("--threads", type=int, nargs="+", default=[1, 8])
    parser.add_argument("--hash", type=int, default=16)
    parser.add_argument("--aggression", type=int, default=75)
    parser.add_argument("--json", type=Path, required=True)
    parser.add_argument("--skip-fixed", action="store_true")
    args = parser.parse_args()
    if args.samples <= 0 or any(t <= 0 for t in args.times_ms) or any(t < 1 or t > 128 for t in args.threads):
        parser.error("positive samples/times and Threads 1..128 are required")
    variants = dict(item.split("=", 1) for item in args.variant)
    if len(variants) != len(args.variant) or len(variants) < 2:
        parser.error("at least two distinct variant labels are required")
    variants = {name: Path(path).resolve(strict=True) for name, path in variants.items()}
    names = list(variants)
    fixtures = parse_suite(args.suite)
    payload = {
        "schema_version": 1,
        "command": sys.argv,
        "host": {"platform": platform.platform(), "affinity": sorted(os.sched_getaffinity(0))},
        "variants": {name: {"path": str(path), "sha256": sha256(path)} for name, path in variants.items()},
        "inputs": {"suite": str(args.suite), "suite_sha256": sha256(args.suite),
                   "script_sha256": sha256(__file__), "uci_harness_sha256": sha256("tools/measure_style.py"),
                   "network_sha256": sha256("nets/jakgro.nnue")},
        "settings": {"threads": args.threads, "times_ms": args.times_ms, "samples": args.samples,
                     "hash_mib": args.hash, "aggression": args.aggression, "use_nnue": True,
                     "order": "rotating/reversing variant order per sample and fixture", "cold_game_per_sample": True},
        "limitations": ["Reported NPS and nodes belong to the last completed iteration, not all work up to bestmove.",
                        "Wall latency measures go write through bestmove receipt, including UCI scheduling and helper joins.",
                        "Bootstrap intervals resample fixture medians; this small suite is not an Elo or broad hardware result."],
        "fixed": [], "timed": [],
    }
    def save():
        args.json.parent.mkdir(parents=True, exist_ok=True)
        temporary = args.json.with_suffix(args.json.suffix + ".tmp")
        temporary.write_text(json.dumps(payload, separators=(",", ":")) + "\n", encoding="utf-8")
        temporary.replace(args.json)
    with ExitStack() as stack:
        engines = {name: stack.enter_context(MeasuredEngine(path, args.hash)) for name, path in variants.items()}
        if not args.skip_fixed:
            for fixture in fixtures:
                for mode in ("nodes", "depth"):
                    for repeat in range(2):
                        for name in names:
                            limits = {} if mode == "nodes" else {"depth": 8}
                            row = engines[name].sample(fixture, args.aggression, threads=1, **limits)
                            payload["fixed"].append({"fixture": fixture.identifier, "mode": mode, "repeat": repeat,
                                                     "variant": name, "signature": signature(row)})
                print("fixed", fixture.identifier, flush=True)
                save()
        for threads in args.threads:
            for milliseconds in args.times_ms:
                for i, fixture in enumerate(fixtures):
                    for name in names:
                        engines[name].sample(fixture, args.aggression, threads=threads, move_time_ms=milliseconds)
                    for sample in range(args.samples):
                        offset = (sample + i) % len(names)
                        order = names[offset:] + names[:offset]
                        if (sample // len(names) + i) % 2:
                            order = list(reversed(order))
                        for name in order:
                            row = engines[name].sample(fixture, args.aggression, threads=threads, move_time_ms=milliseconds)
                            payload["timed"].append({"variant": name, "fixture": fixture.identifier,
                                                     "threads": threads, "movetime_ms": milliseconds, "sample": sample, **row})
                    print("timed", threads, milliseconds, fixture.identifier, flush=True)
                    save()
    payload["fixed_comparisons"] = []
    for candidate, baseline in list(zip(names[1:], names[:-1])) + [(names[-1], names[0])]:
        differences = []
        for fixture in fixtures:
            for mode in ("nodes", "depth"):
                group = [r for r in payload["fixed"] if r["fixture"] == fixture.identifier and r["mode"] == mode]
                left = [r["signature"] for r in group if r["variant"] == candidate]
                right = [r["signature"] for r in group if r["variant"] == baseline]
                if left != right:
                    differences.append({"fixture": fixture.identifier, "mode": mode, "candidate": left, "baseline": right})
        payload["fixed_comparisons"].append({"candidate": candidate, "baseline": baseline, "differences": differences})
    payload["summaries"] = summaries(payload["timed"], names, args.times_ms, args.threads)
    save()
    print(json.dumps({"fixed": payload["fixed_comparisons"], "summaries": [
        {key: value for key, value in summary.items() if key != "fixture_medians"}
        for summary in payload["summaries"]]}, indent=2), flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
