#!/usr/bin/env python3
"""Run a deterministic paired Aggression match through cutechess-cli."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import shlex
import shutil
import subprocess
import sys
from datetime import datetime, timezone
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
    identifiers: set[str] = set()
    for line_number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        try:
            position, identifier = line.split(" id ", 1)
        except ValueError as error:
            raise ValueError(f"{path}:{line_number}: missing opening id") from error
        position = position.rstrip(";").strip()
        if len(position.split()) != 4:
            raise ValueError(f"{path}:{line_number}: expected a four-field EPD position")
        if not identifier.startswith('"') or not identifier.endswith('";'):
            raise ValueError(f"{path}:{line_number}: malformed opening id")
        identifier = identifier[1:-2]
        if not identifier:
            raise ValueError(f"{path}:{line_number}: empty opening id")
        if position in positions:
            raise ValueError(f"{path}:{line_number}: duplicate opening position")
        if identifier in identifiers:
            raise ValueError(f"{path}:{line_number}: duplicate opening id {identifier!r}")
        positions.add(position)
        identifiers.add(identifier)
    if not positions:
        raise ValueError(f"{path}: no opening positions")
    return len(positions)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def resolve_executable(path: Path) -> Path:
    resolved = shutil.which(str(path))
    if resolved is not None:
        return Path(resolved).resolve()
    candidate = path.expanduser().resolve()
    if candidate.is_file() and os.access(candidate, os.X_OK):
        return candidate
    raise ValueError(f"executable does not exist or is not runnable: {path}")


def read_cutechess_version(executable: Path) -> str:
    try:
        result = subprocess.run(
            [str(executable), "--version"],
            check=False,
            capture_output=True,
            text=True,
            timeout=10,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise ValueError(f"cannot query cutechess version: {error}") from error
    output = "\n".join(part.strip() for part in (result.stdout, result.stderr) if part.strip())
    if result.returncode != 0 or not output:
        raise ValueError("cutechess --version did not return a version")
    return output.splitlines()[0]


def count_pgn_games(path: Path) -> int:
    return path.read_text(encoding="utf-8").count("[Event ")


def write_manifest(path: Path, manifest: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(f"{path.suffix}.tmp")
    temporary.write_text(
        f"{json.dumps(manifest, indent=2, sort_keys=True)}\n",
        encoding="utf-8",
    )
    temporary.replace(path)


def build_manifest(
    args: argparse.Namespace,
    command: list[str],
    opening_count: int,
    cutechess_version: str,
) -> dict[str, object]:
    baseline = args.baseline_engine or args.engine
    return {
        "schema_version": 1,
        "created_utc": datetime.now(timezone.utc).isoformat(),
        "command": command,
        "inputs": {
            "runner": {
                "path": str(Path(__file__).resolve()),
                "sha256": sha256_file(Path(__file__).resolve()),
            },
            "candidate": {
                "path": str(args.engine),
                "sha256": sha256_file(args.engine),
                "aggression": args.candidate_aggression,
            },
            "baseline": {
                "path": str(baseline),
                "sha256": sha256_file(baseline),
                "aggression": args.baseline_aggression,
            },
            "openings": {
                "path": str(args.openings),
                "sha256": sha256_file(args.openings),
                "count": opening_count,
            },
            "cutechess": {
                "path": str(args.cutechess),
                "sha256": sha256_file(args.cutechess),
                "version": cutechess_version,
            },
        },
        "settings": {
            "games": args.games,
            "rounds": args.games // 2,
            "nodes_per_move": args.nodes,
            "hash_mib": args.hash,
            "concurrency": 1,
            "draw": {"movenumber": 80, "movecount": 10, "score": 10},
            "resign": {"movecount": 4, "score": 800, "twosided": True},
            "maxmoves": 200,
        },
        "host": {
            "platform": platform.platform(),
            "python": platform.python_version(),
        },
        "outputs": {
            "pgn": str(args.pgn),
            "manifest": str(args.manifest),
        },
    }


def record_execution(
    manifest: dict[str, object],
    args: argparse.Namespace,
    started: datetime,
    finished: datetime,
    return_code: int | None,
    error_message: str | None,
) -> tuple[int, bool]:
    completed_games = count_pgn_games(args.pgn) if args.pgn.is_file() else 0
    complete = return_code == 0 and completed_games == args.games
    manifest["execution"] = {
        "status": "complete" if complete else "failed",
        "started_utc": started.isoformat(),
        "finished_utc": finished.isoformat(),
        "duration_seconds": (finished - started).total_seconds(),
        "return_code": return_code,
        "error": error_message,
        "expected_games": args.games,
        "completed_games": completed_games,
        "pgn_sha256": sha256_file(args.pgn) if args.pgn.is_file() else None,
    }
    return completed_games, complete


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
    parser.add_argument("--games", type=positive, default=96, help="even number of games")
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
    parser.add_argument("--manifest", type=Path, help="output JSON manifest")
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

    if args.dry_run:
        command = build_command(args)
        print(shlex.join(command))
        return 0

    try:
        args.engine = resolve_executable(args.engine)
        args.baseline_engine = (
            resolve_executable(args.baseline_engine)
            if args.baseline_engine is not None
            else args.engine
        )
        args.cutechess = resolve_executable(args.cutechess)
        cutechess_version = read_cutechess_version(args.cutechess)
    except ValueError as error:
        parser.error(str(error))
    args.openings = args.openings.resolve()
    args.pgn = args.pgn.resolve()
    args.manifest = (
        args.manifest.resolve()
        if args.manifest is not None
        else args.pgn.with_suffix(".manifest.json")
    )
    args.pgn.parent.mkdir(parents=True, exist_ok=True)
    args.manifest.parent.mkdir(parents=True, exist_ok=True)
    args.pgn.unlink(missing_ok=True)
    args.manifest.unlink(missing_ok=True)

    command = build_command(args)
    print(shlex.join(command))
    manifest = build_manifest(args, command, opening_count, cutechess_version)
    started = datetime.now(timezone.utc)
    error_message = None
    return_code = None
    try:
        result = subprocess.run(command, check=False)
        return_code = result.returncode
    except OSError as error:
        error_message = str(error)
    finished = datetime.now(timezone.utc)
    completed_games, _ = record_execution(
        manifest,
        args,
        started,
        finished,
        return_code,
        error_message,
    )
    try:
        write_manifest(args.manifest, manifest)
    except OSError as error:
        print(f"run_match: cannot write manifest: {error}", file=sys.stderr)
        return 2

    if error_message is not None:
        print(f"run_match: {error_message}", file=sys.stderr)
        return 2
    if return_code != 0:
        return return_code if return_code is not None and 0 < return_code < 256 else 2
    if completed_games != args.games:
        print(
            f"run_match: expected {args.games} games in PGN, found {completed_games}",
            file=sys.stderr,
        )
        return 3
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
