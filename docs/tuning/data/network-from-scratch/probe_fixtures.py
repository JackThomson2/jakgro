#!/usr/bin/env python3
"""Probe fixture positions at several profiles and node budgets.

usage: probe_fixtures.py ENGINE SUITE.epd ID[,ID...] [--profiles 0,75,100] [--budgets 20000,40000]
       probe_fixtures.py ENGINE --fen "FEN" [--profiles ...] [--budgets ...] [--restrict MOVE]
Prints bestmove/score/depth for each (id, profile, budget), with fresh tables per search.
"""
import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[4] / "tools"))
import measure_style  # noqa: E402


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("engine")
    parser.add_argument("suite", nargs="?")
    parser.add_argument("ids", nargs="?")
    parser.add_argument("--fen")
    parser.add_argument("--profiles", default="0,75,100")
    parser.add_argument("--budgets", default="20000,40000,60000,100000")
    parser.add_argument("--restrict", default=None, help="comma-separated searchmoves for an extra restricted search")
    args = parser.parse_args()
    profiles = [int(p) for p in args.profiles.split(",")]
    budgets = [int(b) for b in args.budgets.split(",")]
    fixtures = []
    if args.fen:
        fixtures.append(measure_style.Fixture("fen", "probe", args.fen, budgets[0], {}))
    else:
        wanted = set(args.ids.split(",")) if args.ids else None
        for fixture in measure_style.parse_suite(Path(args.suite)):
            if wanted is None or fixture.identifier in wanted:
                fixtures.append(fixture)
    engine = measure_style.UciEngine(Path(args.engine), timeout=120.0)
    try:
        for fixture in fixtures:
            print(f"== {fixture.identifier} ({fixture.category}) nodes {fixture.nodes} expected {dict((k, sorted(v)) for k, v in fixture.expected.items())}")
            for profile in profiles:
                row = []
                for budget in budgets:
                    probe = measure_style.Fixture(fixture.identifier, fixture.category, fixture.fen, budget, {})
                    obs = engine.measure(probe, profile)
                    cell = f"{budget//1000}k:{obs.bestmove}({obs.score},d{obs.depth})"
                    if args.restrict:
                        restricted = engine.measure(probe, profile, root_moves=frozenset(args.restrict.split(",")))
                        cell += f"[{args.restrict}:{restricted.score}]"
                    row.append(cell)
                print(f"  A{profile:<3} " + "  ".join(row))
            sys.stdout.flush()
    finally:
        engine.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
