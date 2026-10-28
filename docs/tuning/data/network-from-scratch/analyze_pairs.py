#!/usr/bin/env python3
"""Summarize fastchess rounds without treating a missing FEN as one opening."""

from __future__ import annotations

import argparse
from collections import Counter, defaultdict
import gzip
import hashlib
import json
from pathlib import Path
import random
import re
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parents[4]))
from tools import analyze_match, run_sprt


def ratio_kind(counts: list[int], offset: int) -> str:
    if not counts[0] or not counts[1]:
        return "undefined"
    if not counts[offset + 1]:
        return "unbounded" if counts[offset] else "undefined"
    return "finite"


def ratio(counts: list[int], offset: int) -> float | None:
    candidate_moves, baseline_moves = counts[:2]
    if ratio_kind(counts, offset) != "finite":
        return None
    return counts[offset] * baseline_moves / (counts[offset + 1] * candidate_moves)


def summarize(
    path: Path,
    candidate: str = "Candidate",
    baseline: str = "Baseline",
    book_plies: int = 16,
    bootstrap: int = 4000,
    seed: int = 14875,
    allow_incomplete_pairs: bool = False,
) -> dict:
    if book_plies < 0 or bootstrap < 1 or candidate == baseline:
        raise ValueError("invalid book length, bootstrap count or engine names")
    raw = path.read_bytes()
    if path.suffix == ".gz":
        raw = gzip.decompress(raw)
    text = raw.decode("utf-8")
    with tempfile.TemporaryDirectory() as directory:
        decoded = Path(directory) / "match.pgn"
        decoded.write_bytes(raw)
        games = analyze_match.parse_pgn(decoded)
    rounds = re.findall(r'^\[Round "(\d+)"\]$', text, re.MULTILINE)
    blocks = re.split(r"\n\s*\n(?=\[Event )", text.strip())
    if not games or len(rounds) != len(games) or len(blocks) != len(games):
        raise ValueError("each game must have one numeric Round header")

    grouped = defaultdict(list)
    terminations = Counter()
    recorded_outcomes = Counter()
    white_scores = {"1-0": 1.0, "0-1": 0.0, "1/2-1/2": 0.5}
    for game, round_number, block in zip(games, rounds, blocks):
        if {game.white, game.black} != {candidate, baseline}:
            raise ValueError("unexpected engine names")
        if len(re.findall(r"\{book\}", block)) != book_plies:
            raise ValueError("unexpected number of book plies")
        if len(game.moves) < book_plies:
            raise ValueError("game ends before its opening prefix")
        grouped[int(round_number)].append(game)
        terminations[game.termination] += 1
        point = white_scores[game.result]
        if game.black == candidate:
            point = 1.0 - point
        recorded_outcomes[{0.0: "losses", 0.5: "draws", 1.0: "wins"}[point]] += 1

    pair_points = []
    pair_counts = []
    unpaired_games = []
    outcomes = Counter()
    for round_number, pair in sorted(grouped.items()):
        if len(pair) == 1 and allow_incomplete_pairs:
            game = pair[0]
            unpaired_games.append({
                "round": round_number,
                "white": game.white,
                "black": game.black,
                "result": game.result,
                "termination": game.termination,
            })
            continue
        if len(pair) != 2:
            raise ValueError(f"round {round_number} does not have two games")
        first, second = pair
        if first.white != second.black or first.black != second.white:
            raise ValueError(f"round {round_number} does not reverse colors")
        if first.fen != second.fen or first.moves[:book_plies] != second.moves[:book_plies]:
            raise ValueError(f"round {round_number} has different openings")
        points = 0.0
        counts = [0] * 6  # candidate/baseline: moves, forcing moves, checks
        for game in pair:
            point = white_scores[game.result]
            if game.black == candidate:
                point = 1.0 - point
            points += point
            outcomes[{0.0: "losses", 0.5: "draws", 1.0: "wins"}[point]] += 1
            starts_white = game.fen is None or game.fen.split()[1] == "w"
            for index, move in enumerate(game.moves):
                if index < book_plies:
                    continue
                white_moves = starts_white == (index % 2 == 0)
                player = game.white if white_moves else game.black
                side = 0 if player == candidate else 1
                check = "+" in move or "#" in move
                forcing = check or "x" in move or "=" in move
                counts[side] += 1
                counts[2 + side] += forcing
                counts[4 + side] += check
        pair_points.append(points)
        pair_counts.append(counts)

    if not pair_points:
        raise ValueError("no complete pairs")
    result = run_sprt.evaluate(pair_points, 0.0, 5.0, 0.05, 0.05)
    # The match runner owns the sequential verdict; this estimate is descriptive.
    result.pop("sprt")
    result.pop("los_percent")
    statistics = run_sprt.pair_statistics(pair_points)
    degenerate = statistics["pairs"] < 2 or statistics["variance"] == 0
    result["interval_status"] = "degenerate" if degenerate else "estimated"
    if degenerate:
        result["elo_ci95"] = None
        result["elo_margin_95"] = None
        result["score_percent_ci95"] = None
    totals = [sum(pair[column] for pair in pair_counts) for column in range(6)]
    samples = {"forcing": [], "checks": []}
    replicates = {
        name: Counter({"finite": 0, "undefined": 0, "unbounded": 0})
        for name in samples
    }
    rng = random.Random(seed)
    for _ in range(bootstrap):
        sample = rng.choices(pair_counts, k=len(pair_counts))
        counts = [sum(pair[column] for pair in sample) for column in range(6)]
        for name, offset in [("forcing", 2), ("checks", 4)]:
            replicates[name][ratio_kind(counts, offset)] += 1
            value = ratio(counts, offset)
            if value is not None:
                samples[name].append(value)
    style = {}
    for name, offset in [("forcing", 2), ("checks", 4)]:
        values = sorted(samples[name])
        interval = (
            [values[int(0.025 * (len(values) - 1))], values[int(0.975 * (len(values) - 1))]]
            if len(values) == bootstrap
            else None
        )
        style[name] = {
            "candidate": totals[offset],
            "baseline": totals[offset + 1],
            "rate_ratio": ratio(totals, offset),
            "ratio_status": ratio_kind(totals, offset),
            "bootstrap_ci95": interval,
            "bootstrap_replicates": dict(replicates[name]),
        }
    faults = {
        reason: count
        for reason, count in terminations.items()
        if reason.lower() not in ("normal", "adjudication")
    }
    return {
        "schema_version": 1,
        "pgn": path.name,
        "uncompressed_pgn_sha256": hashlib.sha256(raw).hexdigest(),
        "candidate": candidate,
        "baseline": baseline,
        "book_plies_excluded_per_game": book_plies,
        "recorded_games": len(games),
        "recorded_outcomes": dict(recorded_outcomes),
        "unpaired_games": unpaired_games,
        "strength": {**result, **{name: outcomes[name] for name in ("wins", "draws", "losses")}},
        "style": {
            "candidate_moves": totals[0],
            "baseline_moves": totals[1],
            "bootstrap_samples": bootstrap,
            "bootstrap_seed": seed,
            **style,
        },
        "terminations": dict(terminations),
        "fault_terminations": faults,
        "interval_note": (
            "Unadjusted for sequential stopping and selection across arms. "
            "Degenerate strength intervals and style intervals with singular "
            "bootstrap replicates are unavailable, not zero-width."
        ),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pgn", type=Path)
    parser.add_argument("--candidate", default="Candidate")
    parser.add_argument("--baseline", default="Baseline")
    parser.add_argument("--book-plies", type=int, default=16)
    parser.add_argument("--bootstrap", type=int, default=4000)
    parser.add_argument("--seed", type=int, default=14875)
    parser.add_argument(
        "--allow-incomplete-pairs",
        action="store_true",
        help="report unmatched rounds after a verified normal SPRT stop, excluding them from estimates",
    )
    args = parser.parse_args()
    result = summarize(
        args.pgn, args.candidate, args.baseline, args.book_plies, args.bootstrap, args.seed,
        args.allow_incomplete_pairs,
    )
    print(json.dumps(result, indent=2, sort_keys=True, allow_nan=False))
    return int(bool(result["fault_terminations"]))


if __name__ == "__main__":
    raise SystemExit(main())
