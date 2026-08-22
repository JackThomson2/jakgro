#!/usr/bin/env python3
"""Run a deterministic paired Aggression match through cutechess-cli."""

from __future__ import annotations

import argparse
import shlex
import subprocess
import sys
from pathlib import Path


def aggression(value: str) -> int:
    parsed = int(value)
    if not 0 <= parsed <= 100:
        raise argparse.ArgumentTypeError("Aggression must be between 0 and 100")
    return parsed


def positive(value: str) -> int:
    parsed = int(value)
    if parsed <= 0:
        raise argparse.ArgumentTypeError("value must be positive")
    return parsed


def hash_mib(value: str) -> int:
    parsed = positive(value)
    if parsed > 1024:
        raise argparse.ArgumentTypeError("Hash must be at most 1024 MiB")
    return parsed


def count_openings(path: Path) -> int:
    positions: set[str] = set()
    for line_number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        position = line.split(" id ", 1)[0].rstrip(";").strip()
        if len(position.split()) != 4:
            raise ValueError(f"{path}:{line_number}: expected a four-field EPD position")
        if position in positions:
            raise ValueError(f"{path}:{line_number}: duplicate opening position")
        positions.add(position)
    if not positions:
        raise ValueError(f"{path}: no opening positions")
    return len(positions)


def build_command(args: argparse.Namespace) -> list[str]:
    baseline = args.baseline_engine or args.engine
    return [
        str(args.cutechess),
        "-engine",
        f"cmd={args.engine}",
        f"name=Aggression-{args.candidate_aggression}",
        f"option.Aggression={args.candidate_aggression}",
        "-engine",
        f"cmd={baseline}",
        f"name=Aggression-{args.baseline_aggression}",
        f"option.Aggression={args.baseline_aggression}",
        "-each",
        "proto=uci",
        "tc=inf",
        f"nodes={args.nodes}",
        f"option.Hash={args.hash}",
        "-rounds",
        str(args.games // 2),
        "-games",
        "2",
        "-repeat",
        "-concurrency",
        "1",
        "-openings",
        f"file={args.openings}",
        "format=epd",
        "order=sequential",
        "-draw",
        "movenumber=80",
        "movecount=10",
        "score=10",
        "-resign",
        "movecount=4",
        "score=800",
        "twosided=true",
        "-maxmoves",
        "200",
        "-pgnout",
        str(args.pgn),
    ]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine", type=Path, required=True, help="candidate UCI executable")
    parser.add_argument(
        "--baseline-engine",
        type=Path,
        help="baseline executable; defaults to --engine",
    )
    parser.add_argument("--candidate-aggression", type=aggression, default=100)
    parser.add_argument("--baseline-aggression", type=aggression, default=0)
    parser.add_argument("--games", type=positive, default=12, help="even number of games")
    parser.add_argument("--nodes", type=positive, default=50_000, help="nodes per move")
    parser.add_argument("--hash", type=hash_mib, default=16, help="Hash MiB per engine")
    parser.add_argument(
        "--openings",
        type=Path,
        default=Path("tools/data/openings.epd"),
        help="deterministic EPD opening set",
    )
    parser.add_argument(
        "--pgn",
        type=Path,
        default=Path("artifacts/aggression-match.pgn"),
        help="output PGN",
    )
    parser.add_argument("--cutechess", type=Path, default=Path("cutechess-cli"))
    parser.add_argument("--dry-run", action="store_true", help="print the command only")
    args = parser.parse_args()

    if args.games % 2:
        parser.error("--games must be even so each opening is played with reversed colors")
    if args.candidate_aggression == args.baseline_aggression:
        parser.error("candidate and baseline Aggression values must differ")
    if not args.openings.is_file():
        parser.error(f"opening suite does not exist: {args.openings}")
    try:
        opening_count = count_openings(args.openings)
    except (OSError, ValueError) as error:
        parser.error(str(error))
    if args.games // 2 > opening_count:
        parser.error(
            "opening suite needs at least one unique position per color-reversed pair"
        )
    if not args.dry_run:
        for executable in filter(None, [args.engine, args.baseline_engine]):
            if not executable.is_file():
                parser.error(f"engine does not exist: {executable}")
        args.pgn.parent.mkdir(parents=True, exist_ok=True)

    command = build_command(args)
    print(shlex.join(command))
    if args.dry_run:
        return 0
    try:
        result = subprocess.run(command, check=False)
    except OSError as error:
        print(f"run_match: {error}", file=sys.stderr)
        return 2
    if result.returncode != 0:
        return result.returncode
    try:
        completed_games = args.pgn.read_text(encoding="utf-8").count("[Event ")
    except OSError as error:
        print(f"run_match: cannot read PGN: {error}", file=sys.stderr)
        return 2
    if completed_games != args.games:
        print(
            f"run_match: expected {args.games} games in PGN, found {completed_games}",
            file=sys.stderr,
        )
        return 3
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
