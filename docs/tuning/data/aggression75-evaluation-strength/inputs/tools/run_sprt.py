#!/usr/bin/env python3
"""Run a paired self-play match and evaluate it with a sequential test."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import platform
import shlex
import shutil
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

try:
    from . import analyze_match
except ImportError:  # pragma: no cover - exercised by direct execution
    import analyze_match

SCHEMA_VERSION = 1
#: Pair outcomes in points, from a swept loss to a swept win.
PAIR_OUTCOMES = (0.0, 0.5, 1.0, 1.5, 2.0)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


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


def probability(value: str) -> float:
    parsed = float(value)
    if not 0.0 < parsed < 1.0:
        raise argparse.ArgumentTypeError("value must lie strictly between 0 and 1")
    return parsed


def finite(value: str) -> float:
    parsed = float(value)
    if not math.isfinite(parsed):
        raise argparse.ArgumentTypeError("value must be finite")
    return parsed


def score_from_elo(elo: float) -> float:
    """Returns the expected score for an Elo difference."""
    return 1.0 / (1.0 + 10.0 ** (-elo / 400.0))


def elo_from_score(score: float) -> float | None:
    """Returns the Elo difference implied by a score, or None at the extremes."""
    if score <= 0.0 or score >= 1.0:
        return None
    return 400.0 * math.log10(score / (1.0 - score))


def pair_distribution(pair_points: list[float]) -> list[int]:
    """Counts pair outcomes into the five pentanomial buckets."""
    counts = [0] * len(PAIR_OUTCOMES)
    for points in pair_points:
        index = min(
            range(len(PAIR_OUTCOMES)),
            key=lambda candidate: abs(PAIR_OUTCOMES[candidate] - points),
        )
        if abs(PAIR_OUTCOMES[index] - points) > 1e-9:
            raise ValueError(f"pair score {points} is not a multiple of half a point")
        counts[index] += 1
    return counts


def pair_statistics(pair_points: list[float]) -> dict[str, float]:
    """Returns the mean pair score and its variance, both per game."""
    if not pair_points:
        raise ValueError("a paired match needs at least one pair")
    scores = [points / 2.0 for points in pair_points]
    pairs = len(scores)
    mean = sum(scores) / pairs
    if pairs < 2:
        return {"pairs": pairs, "mean": mean, "variance": 0.0, "sigma": 0.0}
    variance = sum((score - mean) ** 2 for score in scores) / (pairs - 1)
    return {
        "pairs": pairs,
        "mean": mean,
        "variance": variance,
        "sigma": math.sqrt(variance / pairs),
    }


def normal_interval(statistics: dict[str, float], confidence: float = 0.95) -> tuple[float, float]:
    """Returns a paired normal confidence interval for the mean score."""
    quantiles = {0.95: 1.959963984540054, 0.99: 2.5758293035489004}
    z = quantiles.get(round(confidence, 4), 1.959963984540054)
    margin = z * statistics["sigma"]
    return (
        max(0.0, statistics["mean"] - margin),
        min(1.0, statistics["mean"] + margin),
    )


def log_likelihood_ratio(
    pair_points: list[float],
    elo0: float,
    elo1: float,
) -> float:
    """Returns the generalized log-likelihood ratio for a paired match.

    This is the standard normal-approximation form used for pentanomial SPRT: the
    two hypotheses are expressed as expected scores, and the ratio compares them
    through the observed pair mean and variance. It reduces to zero when the two
    hypotheses coincide and when no variance has been observed yet, which keeps
    an undecided test undecided rather than guessing.
    """
    statistics = pair_statistics(pair_points)
    variance = statistics["variance"]
    if variance <= 0.0 or elo0 == elo1:
        return 0.0
    score0 = score_from_elo(elo0)
    score1 = score_from_elo(elo1)
    mean = statistics["mean"]
    pairs = statistics["pairs"]
    return (
        pairs
        * (score1 - score0)
        * (2.0 * mean - score0 - score1)
        / (2.0 * variance)
    )


def sprt_decision(llr: float, alpha: float, beta: float) -> dict[str, Any]:
    """Compares a log-likelihood ratio against Wald's stopping boundaries."""
    lower = math.log(beta / (1.0 - alpha))
    upper = math.log((1.0 - beta) / alpha)
    if llr >= upper:
        decision = "accept_h1"
    elif llr <= lower:
        decision = "accept_h0"
    else:
        decision = "continue"
    return {
        "llr": round(llr, 6),
        "lower_bound": round(lower, 6),
        "upper_bound": round(upper, 6),
        "decision": decision,
    }


def evaluate(
    pair_points: list[float],
    elo0: float,
    elo1: float,
    alpha: float,
    beta: float,
) -> dict[str, Any]:
    """Summarizes a paired match as scores, Elo bounds, and an SPRT verdict."""
    statistics = pair_statistics(pair_points)
    low, high = normal_interval(statistics)
    llr = log_likelihood_ratio(pair_points, elo0, elo1)
    elo = elo_from_score(statistics["mean"])
    elo_low = elo_from_score(low)
    elo_high = elo_from_score(high)
    return {
        "pairs": statistics["pairs"],
        "games": statistics["pairs"] * 2,
        "pair_distribution": dict(
            zip(
                (f"{outcome:.1f}" for outcome in PAIR_OUTCOMES),
                pair_distribution(pair_points),
            )
        ),
        "score_percent": round(statistics["mean"] * 100.0, 6),
        "score_percent_ci95": [round(low * 100.0, 6), round(high * 100.0, 6)],
        "elo": None if elo is None else round(elo, 6),
        "elo_ci95": [
            None if elo_low is None else round(elo_low, 6),
            None if elo_high is None else round(elo_high, 6),
        ],
        "elo_margin_95": (
            None
            if elo is None or elo_low is None or elo_high is None
            else round((elo_high - elo_low) / 2.0, 6)
        ),
        "los_percent": round(los(statistics) * 100.0, 6),
        "sprt": {
            "elo0": elo0,
            "elo1": elo1,
            "alpha": alpha,
            "beta": beta,
            **sprt_decision(llr, alpha, beta),
        },
    }


def los(statistics: dict[str, float]) -> float:
    """Returns the likelihood that the candidate is stronger than the baseline."""
    sigma = statistics["sigma"]
    if sigma <= 0.0:
        return 1.0 if statistics["mean"] > 0.5 else 0.0
    z = (statistics["mean"] - 0.5) / sigma
    return 0.5 * (1.0 + math.erf(z / math.sqrt(2.0)))


def pair_points_from_pgn(pgn: Path, candidate: str, baseline: str) -> list[float]:
    """Reads colour-reversed pair scores back out of a played PGN.

    The arbiter's own totals are deliberately not trusted here: parsing the PGN
    independently means a bookkeeping error in the runner cannot silently become
    a strength claim.
    """
    games = analyze_match.parse_pgn(pgn)
    if len(games) % 2:
        raise ValueError("paired match has an odd game count")
    points: list[float] = []
    for index in range(0, len(games), 2):
        first, second = games[index : index + 2]
        if first.white != second.black or first.black != second.white:
            raise ValueError(f"games {index + 1}-{index + 2} are not color-reversed")
        if first.fen != second.fen:
            raise ValueError(f"games {index + 1}-{index + 2} do not share an opening FEN")
        points.append(
            sum(
                analyze_match.candidate_points(game, candidate, baseline)
                for game in (first, second)
            )
        )
    return points


def engine_names(args: argparse.Namespace) -> tuple[str, str]:
    candidate = args.candidate_name or f"Aggression-{args.candidate_aggression}"
    baseline = args.baseline_name or f"Aggression-{args.baseline_aggression}"
    if candidate == baseline:
        if args.candidate_name or args.baseline_name:
            raise ValueError("candidate and baseline engine names must differ")
        candidate = f"Candidate-{candidate}"
        baseline = f"Baseline-{baseline}"
    return candidate, baseline


def resolve_executable(path: Path) -> Path:
    resolved = shutil.which(str(path))
    if resolved is not None:
        return Path(resolved).resolve()
    candidate = path.expanduser().resolve()
    if candidate.is_file():
        return candidate
    raise ValueError(f"executable does not exist: {path}")


def build_command(args: argparse.Namespace, candidate: str, baseline: str) -> list[str]:
    command = [
        str(args.runner),
        "--engine",
        str(args.engine),
        "--baseline-engine",
        str(args.baseline_engine),
        "--candidate-aggression",
        str(args.candidate_aggression),
        "--baseline-aggression",
        str(args.baseline_aggression),
        "--candidate-name",
        candidate,
        "--baseline-name",
        baseline,
        "--games",
        str(args.games),
        "--openings",
        str(args.openings),
        "--hash",
        str(args.hash),
        "--concurrency",
        str(args.concurrency),
        "--pgn",
        str(args.pgn),
        "--results-json",
        str(args.results_json),
    ]
    if args.time_control is not None:
        command += ["--time-control", args.time_control]
    elif args.movetime_ms is not None:
        command += ["--movetime-ms", str(args.movetime_ms)]
    else:
        command += ["--nodes", str(args.nodes)]
    return command


def build_manifest(
    args: argparse.Namespace,
    command: list[str],
    candidate: str,
    baseline: str,
    opening_count: int,
) -> dict[str, Any]:
    candidate_hash = sha256_file(args.engine)
    baseline_hash = sha256_file(args.baseline_engine)
    identical = candidate_hash == baseline_hash
    if (
        args.candidate_aggression == args.baseline_aggression
        and identical
        and not args.allow_identical_binaries
    ):
        raise ValueError(
            "same-profile candidate and baseline binaries must differ; "
            "pass --allow-identical-binaries for a harness control run"
        )
    limit_mode = (
        "fixed-time"
        if args.time_control is not None
        else "fixed-movetime"
        if args.movetime_ms is not None
        else "fixed-nodes"
    )
    return {
        "schema_version": 2,
        "created_utc": datetime.now(timezone.utc).isoformat(),
        "command": command,
        "comparison": {
            "mode": (
                "same-profile-binaries"
                if args.candidate_aggression == args.baseline_aggression
                else "profile-self-play"
            ),
            "distinct_binaries_required": (
                args.candidate_aggression == args.baseline_aggression
                and not args.allow_identical_binaries
            ),
            "distinct_binary_hashes": candidate_hash != baseline_hash,
            "identical_binaries_allowed": bool(args.allow_identical_binaries),
        },
        "inputs": {
            "runner": {
                "path": str(args.runner),
                "sha256": sha256_file(args.runner),
            },
            "harness": {
                "path": str(Path(__file__).resolve()),
                "sha256": sha256_file(Path(__file__).resolve()),
            },
            "candidate": {
                "name": candidate,
                "path": str(args.engine),
                "sha256": candidate_hash,
                "aggression": args.candidate_aggression,
                "revision": args.candidate_revision,
            },
            "baseline": {
                "name": baseline,
                "path": str(args.baseline_engine),
                "sha256": baseline_hash,
                "aggression": args.baseline_aggression,
                "revision": args.baseline_revision,
            },
            "openings": {
                "path": str(args.openings),
                "sha256": sha256_file(args.openings),
                "count": opening_count,
            },
        },
        "provenance": {
            "build_profile": args.build_profile,
            "dependency_revision": args.dependency_revision,
        },
        "settings": {
            "games": args.games,
            "rounds": args.games // 2,
            "limit": {
                "mode": limit_mode,
                "nodes_per_move": args.nodes if limit_mode == "fixed-nodes" else None,
                "movetime_ms": args.movetime_ms,
                "time_control": args.time_control,
            },
            "nodes_per_move": args.nodes if limit_mode == "fixed-nodes" else None,
            "time_control": args.time_control,
            "hash_mib": args.hash,
            "concurrency": args.concurrency,
            "draw": {"movenumber": 80, "movecount": 10, "score": 10},
            "resign": {"movecount": 4, "score": 800, "twosided": True},
            "maxmoves": 200,
            "sprt": {
                "elo0": args.elo0,
                "elo1": args.elo1,
                "alpha": args.alpha,
                "beta": args.beta,
            },
        },
        "host": {
            "platform": platform.platform(),
            "python": platform.python_version(),
        },
        "outputs": {"pgn": str(args.pgn), "manifest": str(args.manifest)},
    }


def record_execution(
    manifest: dict[str, Any],
    args: argparse.Namespace,
    started: datetime,
    finished: datetime,
    return_code: int | None,
    error_message: str | None,
) -> int:
    completed_games = len(analyze_match.parse_pgn(args.pgn)) if args.pgn.is_file() else 0
    faults = arbiter_faults(args.results_json)
    if faults and error_message is None:
        error_message = f"{len(faults)} game(s) ended in a fault"
    complete = return_code == 0 and completed_games == args.games and error_message is None
    manifest["execution"] = {
        "status": "complete" if complete else "failed",
        "started_utc": started.isoformat(),
        "finished_utc": finished.isoformat(),
        "duration_seconds": (finished - started).total_seconds(),
        "return_code": return_code,
        "error": error_message,
        "expected_games": args.games,
        "completed_games": completed_games,
        "faults": faults,
        "inputs_unchanged": True,
        "changed_inputs": [],
        "pgn_sha256": sha256_file(args.pgn) if args.pgn.is_file() else None,
    }
    return completed_games


def arbiter_faults(path: Path) -> list[dict[str, Any]]:
    """Reads the protocol, legality, and timing faults the runner recorded."""
    if not path.is_file():
        return []
    try:
        recorded = json.loads(path.read_text(encoding="utf-8")).get("faults", [])
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"cannot read arbiter results {path}: {error}") from error
    if not isinstance(recorded, list):
        raise ValueError(f"{path}: faults must be a list")
    return recorded


def write_json(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(f"{path.suffix}.tmp")
    temporary.write_text(
        f"{json.dumps(payload, indent=2, sort_keys=True)}\n", encoding="utf-8"
    )
    temporary.replace(path)


def parse_arguments(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--baseline-engine", type=Path)
    parser.add_argument(
        "--runner",
        type=Path,
        default=Path("target/release/selfplay"),
        help="paired match arbiter built from src/bin/selfplay.rs",
    )
    parser.add_argument("--candidate-aggression", type=aggression, default=75)
    parser.add_argument("--baseline-aggression", type=aggression, default=75)
    parser.add_argument("--candidate-name")
    parser.add_argument("--baseline-name")
    parser.add_argument("--games", type=positive, default=1000)
    limits = parser.add_mutually_exclusive_group()
    limits.add_argument("--nodes", type=positive)
    limits.add_argument("--movetime-ms", type=positive)
    limits.add_argument("--time-control")
    parser.add_argument("--hash", type=positive, default=16)
    parser.add_argument("--concurrency", type=positive, default=8)
    parser.add_argument(
        "--openings",
        type=Path,
        default=Path("docs/tuning/data/selective-search-confirmation.epd"),
        help="sequential EPD suite; needs one unique position per color-reversed pair",
    )
    parser.add_argument("--pgn", type=Path, default=Path("artifacts/sprt.pgn"))
    parser.add_argument("--manifest", type=Path)
    parser.add_argument("--summary-json", type=Path)
    parser.add_argument("--results-json", type=Path)
    parser.add_argument("--elo0", type=finite, default=0.0)
    parser.add_argument("--elo1", type=finite, default=10.0)
    parser.add_argument("--alpha", type=probability, default=0.05)
    parser.add_argument("--beta", type=probability, default=0.05)
    parser.add_argument("--candidate-revision")
    parser.add_argument("--baseline-revision")
    parser.add_argument("--dependency-revision")
    parser.add_argument("--build-profile")
    parser.add_argument(
        "--allow-identical-binaries",
        action="store_true",
        help="permit one binary on both sides, for harness control runs",
    )
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args(argv)
    if args.games % 2:
        parser.error("--games must be even so each opening is played with reversed colors")
    if args.elo1 <= args.elo0:
        parser.error("--elo1 must exceed --elo0")
    if args.nodes is None and args.movetime_ms is None and args.time_control is None:
        args.nodes = 50_000
    return args


def main(argv: list[str] | None = None) -> int:
    args = parse_arguments(argv)
    args.baseline_engine = args.baseline_engine or args.engine
    try:
        candidate, baseline = engine_names(args)
    except ValueError as error:
        print(f"run_sprt: {error}", file=sys.stderr)
        return 2
    args.pgn = args.pgn.resolve()
    args.manifest = (
        args.manifest.resolve()
        if args.manifest is not None
        else args.pgn.with_suffix(".manifest.json")
    )
    args.summary_json = (
        args.summary_json.resolve()
        if args.summary_json is not None
        else args.pgn.with_suffix(".sprt.json")
    )
    args.results_json = (
        args.results_json.resolve()
        if args.results_json is not None
        else args.pgn.with_suffix(".arbiter.json")
    )

    try:
        args.runner = resolve_executable(args.runner)
        args.engine = resolve_executable(args.engine)
        args.baseline_engine = resolve_executable(args.baseline_engine)
        args.openings = args.openings.resolve(strict=True)
    except (OSError, ValueError) as error:
        print(f"run_sprt: {error}", file=sys.stderr)
        return 2

    command = build_command(args, candidate, baseline)
    print(shlex.join(command))
    if args.dry_run:
        return 0

    try:
        opening_count = sum(
            1
            for line in args.openings.read_text(encoding="utf-8").splitlines()
            if line.strip() and not line.strip().startswith("#")
        )
        manifest = build_manifest(args, command, candidate, baseline, opening_count)
    except (OSError, ValueError) as error:
        print(f"run_sprt: {error}", file=sys.stderr)
        return 2

    args.pgn.parent.mkdir(parents=True, exist_ok=True)
    args.pgn.unlink(missing_ok=True)
    started = datetime.now(timezone.utc)
    return_code: int | None = None
    error_message: str | None = None
    try:
        return_code = subprocess.run(command, check=False).returncode
    except OSError as error:  # pragma: no cover - depends on the host
        error_message = str(error)
    finished = datetime.now(timezone.utc)
    if not args.pgn.is_file():
        print(
            f"run_sprt: the runner wrote no PGN (exit {return_code}); "
            "see its diagnostics above",
            file=sys.stderr,
        )
        return 2

    try:
        completed_games = record_execution(
            manifest, args, started, finished, return_code, error_message
        )
        write_json(args.manifest, manifest)
        pair_points = pair_points_from_pgn(args.pgn, candidate, baseline)
        evaluation = evaluate(pair_points, args.elo0, args.elo1, args.alpha, args.beta)
    except (OSError, ValueError) as error:
        print(f"run_sprt: {error}", file=sys.stderr)
        return 2

    summary = {
        "schema_version": SCHEMA_VERSION,
        "engines": {"candidate": candidate, "baseline": baseline},
        "profiles": {
            "candidate": args.candidate_aggression,
            "baseline": args.baseline_aggression,
        },
        "inputs": {
            "pgn": args.pgn.name,
            "pgn_sha256": sha256_file(args.pgn),
            "manifest": args.manifest.name,
            "manifest_sha256": sha256_file(args.manifest),
            "candidate_sha256": manifest["inputs"]["candidate"]["sha256"],
            "baseline_sha256": manifest["inputs"]["baseline"]["sha256"],
            "openings_sha256": manifest["inputs"]["openings"]["sha256"],
        },
        "limit": manifest["settings"]["limit"],
        "status": manifest["execution"]["status"],
        "faults": manifest["execution"]["faults"],
        "result": evaluation,
    }
    write_json(args.summary_json, summary)
    verdict = evaluation["sprt"]["decision"]
    print(
        f"run_sprt: {evaluation['games']} games, "
        f"{evaluation['score_percent']:.2f}%, "
        f"Elo {evaluation['elo']} "
        f"[{evaluation['elo_ci95'][0]}, {evaluation['elo_ci95'][1]}], "
        f"LLR {evaluation['sprt']['llr']:.3f} -> {verdict}"
    )
    if manifest["execution"]["status"] != "complete":
        print(
            f"run_sprt: match did not complete cleanly: "
            f"{manifest['execution']['error']} "
            f"({completed_games}/{args.games} games)",
            file=sys.stderr,
        )
        return 3
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
