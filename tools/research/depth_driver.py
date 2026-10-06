#!/usr/bin/env python3
"""Fixed-depth or fixed-time totals over book positions, per UCI option set.

usage: depth_driver.py ENGINE MODE VALUE COUNT [--opt NAME=VALUE ...]
                       [--hash MIB] [--skip N] [--book EPD] [--per-position]

MODE is ``depth`` or ``movetime``. The positions are the first COUNT entries of
the book after a fixed shuffle (seed 7), so the 40-position depth-13 totals
24,857,646 / 26,995,867 / 49,930,907 nodes at Aggression 0 / 75 / 100 serve
as the engine's tree-identity signature. Prints total nodes, total reported
milliseconds, mean completed depth and wall-clock statistics.
"""
import argparse
import random
import statistics
import subprocess
import time
from pathlib import Path

DEFAULT_BOOK = (
    Path(__file__).resolve().parents[2]
    / "docs"
    / "tuning"
    / "data"
    / "selective-search-confirmation.epd"
)


def read_book(path, skip, count):
    fens = []
    for line in Path(path).read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        fields = line.split(";")[0].split()
        if len(fields) >= 4:
            fens.append(" ".join(fields[:4]) + " 0 1")
    random.Random(7).shuffle(fens)
    return fens[skip : skip + count]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("engine")
    parser.add_argument("mode", choices=["depth", "movetime"])
    parser.add_argument("value", type=int)
    parser.add_argument("count", type=int)
    parser.add_argument("--opt", action="append", default=[], metavar="NAME=VALUE")
    parser.add_argument("--hash", type=int, default=16)
    parser.add_argument("--skip", type=int, default=0)
    parser.add_argument("--book", default=str(DEFAULT_BOOK))
    parser.add_argument("--per-position", action="store_true")
    args = parser.parse_args()

    fens = read_book(args.book, args.skip, args.count)
    process = subprocess.Popen(
        [args.engine],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        text=True,
        bufsize=1,
    )

    def send(command):
        process.stdin.write(command + "\n")
        process.stdin.flush()

    def until(token):
        lines = []
        while True:
            line = process.stdout.readline()
            if not line:
                raise SystemExit("engine closed")
            line = line.strip()
            lines.append(line)
            if line == token or line.startswith(token + " "):
                return lines

    send("uci")
    until("uciok")
    send(f"setoption name Hash value {args.hash}")
    for option in args.opt:
        name, value = option.split("=", 1)
        send(f"setoption name {name} value {value}")
    send("isready")
    until("readyok")

    rows = []
    for fen in fens:
        send("ucinewgame")
        send("isready")
        until("readyok")
        send(f"position fen {fen}")
        started = time.time()
        send(f"go {args.mode} {args.value}")
        last = None
        for line in until("bestmove"):
            fields = line.split()
            if fields[:1] == ["info"] and "nodes" in fields and "depth" in fields and "score" in fields:
                last = fields
        wall_ms = (time.time() - started) * 1000
        if last is None:
            continue
        nodes = int(last[last.index("nodes") + 1])
        depth = int(last[last.index("depth") + 1])
        elapsed = int(last[last.index("time") + 1])
        score_index = last.index("score")
        if last[score_index + 1] == "cp":
            score = int(last[score_index + 2])
        else:
            score = 10000 if int(last[score_index + 2]) > 0 else -10000
        rows.append((nodes, elapsed, depth, score, wall_ms))
    send("quit")
    try:
        process.wait(timeout=30)
    except subprocess.TimeoutExpired:
        process.kill()

    if args.per_position:
        for row in rows:
            print(row)
    total_nodes = sum(row[0] for row in rows)
    total_ms = sum(row[1] for row in rows)
    print(
        f"opts={args.opt} mode={args.mode} {args.value} positions={len(rows)} "
        f"total_nodes={total_nodes} total_ms={total_ms} "
        f"mean_depth={statistics.mean(row[2] for row in rows):.2f} "
        f"mean_wall_ms={statistics.mean(row[4] for row in rows):.0f} "
        f"max_wall_ms={max(row[4] for row in rows):.0f}"
    )


if __name__ == "__main__":
    main()
