#!/usr/bin/env python3
"""Validate and summarize a paired Aggression match PGN."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import re
import sys
from collections import Counter
from dataclasses import dataclass
from pathlib import Path

HEADER = re.compile(r'^\[([A-Za-z][A-Za-z0-9_]*) "((?:\\.|[^"\\])*)"\]$')
VALID_RESULTS = {"1-0", "0-1", "1/2-1/2"}


@dataclass(frozen=True)
class Game:
    event: str
    white: str
    black: str
    result: str
    termination: str
    fen: str | None
    ply_count: int | None

def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def unescape_header(value: str) -> str:
    return value.replace(r"\"", '"').replace(r"\\", "\\")


def parse_pgn(path: Path) -> list[Game]:
    games: list[Game] = []
    headers: dict[str, str] = {}

    def finish_game() -> None:
        nonlocal headers
        if not headers:
            return
        missing = [key for key in ("Event", "White", "Black", "Result") if key not in headers]
        if missing:
            raise ValueError(f"game {len(games) + 1} is missing headers: {', '.join(missing)}")
        result = headers["Result"]
        if result not in VALID_RESULTS:
            raise ValueError(f"game {len(games) + 1} has incomplete result {result!r}")
        ply_count = None
        if "PlyCount" in headers:
            try:
                ply_count = int(headers["PlyCount"])
            except ValueError as error:
                raise ValueError(f"game {len(games) + 1} has invalid PlyCount") from error
            if ply_count < 0:
                raise ValueError(f"game {len(games) + 1} has negative PlyCount")
        games.append(
            Game(
                event=headers["Event"],
                white=headers["White"],
                black=headers["Black"],
                result=result,
                termination=headers.get("Termination", "unknown"),
                fen=headers.get("FEN"),
                ply_count=ply_count,
            )
        )
        headers = {}

    for line_number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.strip()
        if not line.startswith("["):
            continue
        match = HEADER.fullmatch(line)
        if match is None:
            raise ValueError(f"{path}:{line_number}: malformed PGN header")
        key, value = match.groups()
        if key == "Event" and headers:
            finish_game()
        if key in headers:
            raise ValueError(f"{path}:{line_number}: duplicate {key} header")
        headers[key] = unescape_header(value)
    finish_game()
    if not games:
        raise ValueError(f"{path}: no games")
    return games


def load_manifest(path: Path, pgn: Path) -> tuple[dict[str, object], str, str, int]:
    try:
        manifest = json.loads(path.read_text(encoding="utf-8"))
        candidate = manifest["inputs"]["candidate"]
        baseline = manifest["inputs"]["baseline"]
        execution = manifest["execution"]
        expected_games = int(manifest["settings"]["games"])
        candidate_name = f"Aggression-{int(candidate['aggression'])}"
        baseline_name = f"Aggression-{int(baseline['aggression'])}"
    except (OSError, KeyError, TypeError, ValueError, json.JSONDecodeError) as error:
        raise ValueError(f"invalid match manifest {path}: {error}") from error
    if manifest.get("schema_version") != 1:
        raise ValueError("unsupported match manifest schema")
    if execution.get("status") != "complete":
        raise ValueError("match manifest does not describe a completed run")
    if int(execution.get("completed_games", -1)) != expected_games:
        raise ValueError("manifest game count is incomplete")
    expected_hash = execution.get("pgn_sha256")
    actual_hash = sha256_file(pgn)
    if expected_hash != actual_hash:
        raise ValueError("PGN hash does not match the manifest")
    if candidate_name == baseline_name:
        raise ValueError("candidate and baseline names are identical")
    return manifest, candidate_name, baseline_name, expected_games


def candidate_points(game: Game, candidate: str, baseline: str) -> float:
    players = {game.white, game.black}
    if players != {candidate, baseline}:
        raise ValueError(
            f"game uses unexpected players {game.white!r} and {game.black!r}"
        )
    if game.result == "1/2-1/2":
        return 0.5
    winner = game.white if game.result == "1-0" else game.black
    return 1.0 if winner == candidate else 0.0


def result_counts(points: list[float]) -> dict[str, int]:
    return {
        "wins": sum(point == 1.0 for point in points),
        "draws": sum(point == 0.5 for point in points),
        "losses": sum(point == 0.0 for point in points),
    }


def percentage(value: float) -> float:
    return round(value * 100.0, 6)


def elo_from_score(score: float) -> float | None:
    if score <= 0.0 or score >= 1.0:
        return None
    return round(400.0 * math.log10(score / (1.0 - score)), 6)


def paired_score_interval(pair_scores: list[float]) -> tuple[float, float]:
    if len(pair_scores) < 2:
        return 0.0, 1.0
    mean = sum(pair_scores) / len(pair_scores)
    variance = sum((score - mean) ** 2 for score in pair_scores) / (len(pair_scores) - 1)
    standard_error = math.sqrt(variance / len(pair_scores))
    margin = 1.96 * standard_error
    return max(0.0, mean - margin), min(1.0, mean + margin)


def summarize(
    games: list[Game],
    manifest: dict[str, object],
    candidate: str,
    baseline: str,
    pgn: Path,
    manifest_path: Path,
) -> dict[str, object]:
    expected_games = int(manifest["settings"]["games"])
    if len(games) != expected_games:
        raise ValueError(f"expected {expected_games} games, found {len(games)}")
    if len(games) % 2:
        raise ValueError("paired match has an odd game count")

    points = [candidate_points(game, candidate, baseline) for game in games]
    white_points = [
        point for game, point in zip(games, points) if game.white == candidate
    ]
    black_points = [
        point for game, point in zip(games, points) if game.black == candidate
    ]
    pair_scores: list[float] = []
    pair_distribution: Counter[str] = Counter()
    double_draws = 0
    decisive_splits = 0
    for index in range(0, len(games), 2):
        first, second = games[index : index + 2]
        if first.white != second.black or first.black != second.white:
            raise ValueError(f"games {index + 1}-{index + 2} are not color-reversed")
        if first.fen != second.fen:
            raise ValueError(f"games {index + 1}-{index + 2} do not share an opening FEN")
        first_point, second_point = points[index : index + 2]
        pair_points = first_point + second_point
        pair_scores.append(pair_points / 2.0)
        pair_distribution[f"{pair_points:.1f}"] += 1
        double_draws += int(first_point == 0.5 and second_point == 0.5)
        decisive_splits += int({first_point, second_point} == {0.0, 1.0})

    score = sum(points) / len(points)
    low, high = paired_score_interval(pair_scores)
    counts = result_counts(points)
    white_counts = result_counts(white_points)
    black_counts = result_counts(black_points)
    white_score = sum(white_points) / len(white_points)
    black_score = sum(black_points) / len(black_points)
    plies = [game.ply_count for game in games if game.ply_count is not None]
    terminations = Counter(game.termination for game in games)

    return {
        "schema_version": 1,
        "inputs": {
            "pgn": pgn.name,
            "pgn_sha256": sha256_file(pgn),
            "manifest": manifest_path.name,
            "manifest_sha256": sha256_file(manifest_path),
        },
        "engines": {"candidate": candidate, "baseline": baseline},
        "games": len(games),
        "pairs": {
            "count": len(pair_scores),
            "point_distribution": dict(sorted(pair_distribution.items())),
            "double_draws": double_draws,
            "decisive_splits": decisive_splits,
        },
        "result": {
            **counts,
            "score_percent": percentage(score),
            "decisive_percent": percentage((counts["wins"] + counts["losses"]) / len(games)),
        },
        "colors": {
            "white": {**white_counts, "score_percent": percentage(white_score)},
            "black": {**black_counts, "score_percent": percentage(black_score)},
            "score_gap_percentage_points": round(
                percentage(white_score) - percentage(black_score), 6
            ),
        },
        "confidence": {
            "method": "normal interval over color-reversed pair scores",
            "score_percent_ci95": [percentage(low), percentage(high)],
            "elo": elo_from_score(score),
            "elo_ci95": [elo_from_score(low), elo_from_score(high)],
        },
        "terminations": dict(sorted(terminations.items())),
        "average_plies": round(sum(plies) / len(plies), 6) if plies else None,
    }


def format_elo(value: float | None, lower: bool = False) -> str:
    if value is None:
        return "-infinity" if lower else "+infinity"
    return f"{value:+.1f}"


def markdown(summary: dict[str, object]) -> str:
    result = summary["result"]
    colors = summary["colors"]
    confidence = summary["confidence"]
    pairs = summary["pairs"]
    lines = [
        "# Aggression paired-match summary",
        "",
        f"- Candidate: `{summary['engines']['candidate']}`",
        f"- Baseline: `{summary['engines']['baseline']}`",
        f"- Games: {summary['games']} ({pairs['count']} color-reversed pairs)",
        f"- W/D/L: {result['wins']}/{result['draws']}/{result['losses']}",
        f"- Score: {result['score_percent']:.2f}%",
        f"- Decisive games: {result['decisive_percent']:.2f}%",
        (
            "- Approximate Elo: "
            f"{format_elo(confidence['elo'], lower=result['score_percent'] < 50.0)}"
        ),
        (
            "- Approximate 95% score interval: "
            f"{confidence['score_percent_ci95'][0]:.2f}% to "
            f"{confidence['score_percent_ci95'][1]:.2f}%"
        ),
        (
            "- Approximate 95% Elo interval: "
            f"{format_elo(confidence['elo_ci95'][0], lower=True)} to "
            f"{format_elo(confidence['elo_ci95'][1])}"
        ),
        "",
        "## Color split",
        "",
        "| Candidate color | W | D | L | Score |",
        "| --- | ---: | ---: | ---: | ---: |",
        (
            f"| White | {colors['white']['wins']} | {colors['white']['draws']} | "
            f"{colors['white']['losses']} | {colors['white']['score_percent']:.2f}% |"
        ),
        (
            f"| Black | {colors['black']['wins']} | {colors['black']['draws']} | "
            f"{colors['black']['losses']} | {colors['black']['score_percent']:.2f}% |"
        ),
        "",
        "## Pair outcomes",
        "",
        f"- Point distribution: `{json.dumps(pairs['point_distribution'], sort_keys=True)}`",
        f"- Double draws: {pairs['double_draws']}",
        f"- Decisive splits: {pairs['decisive_splits']}",
        "",
        "## Terminations",
        "",
        "| Termination | Games |",
        "| --- | ---: |",
    ]
    lines.extend(
        f"| {termination} | {count} |"
        for termination, count in summary["terminations"].items()
    )
    lines.extend(
        [
            "",
            "## Reproducibility",
            "",
            f"- PGN SHA-256: `{summary['inputs']['pgn_sha256']}`",
            f"- Manifest SHA-256: `{summary['inputs']['manifest_sha256']}`",
            "",
            (
                "The confidence interval is an approximate normal interval over paired opening "
                "scores, not an SPRT result."
            ),
            "",
        ]
    )
    return "\n".join(lines)


def write_text(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(f"{path.suffix}.tmp")
    temporary.write_text(content, encoding="utf-8")
    temporary.replace(path)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pgn", type=Path, required=True)
    parser.add_argument("--manifest", type=Path)
    parser.add_argument("--json", type=Path, help="write deterministic JSON summary")
    parser.add_argument("--markdown", type=Path, help="write Markdown summary")
    args = parser.parse_args()
    manifest_path = args.manifest or args.pgn.with_suffix(".manifest.json")

    try:
        manifest, candidate, baseline, _ = load_manifest(manifest_path, args.pgn)
        games = parse_pgn(args.pgn)
        summary = summarize(games, manifest, candidate, baseline, args.pgn, manifest_path)
        rendered = markdown(summary)
        if args.json is not None:
            write_text(
                args.json,
                f"{json.dumps(summary, indent=2, sort_keys=True)}\n",
            )
        if args.markdown is not None:
            write_text(args.markdown, rendered)
        if args.json is None and args.markdown is None:
            print(rendered, end="")
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"analyze_match: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
