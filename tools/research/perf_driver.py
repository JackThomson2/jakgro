#!/usr/bin/env python3
"""Fixed-node throughput over book positions, optionally under ``perf``.

usage: perf_driver.py ENGINE NODES COUNT [--perf OUT.data] [--stat]
                      [--hash MIB] [--aggression N] [--book EPD] [--freq HZ]

Prints the median per-position NPS, the mean completed depth and the aggregate
NPS. With ``--perf`` the engine runs under ``perf record`` (inspect with
``perf report -i OUT.data --no-children --stdio``); with ``--stat`` under
``perf stat`` for cycles, instructions, branches and cache misses.
"""
import argparse
import random
import statistics
import subprocess
from pathlib import Path

DEFAULT_BOOK = (
    Path(__file__).resolve().parents[2]
    / "docs"
    / "tuning"
    / "data"
    / "selective-search-confirmation.epd"
)


def read_book(path, count):
    fens = []
    for line in Path(path).read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        fields = line.split(";")[0].split()
        if len(fields) >= 4:
            fens.append(" ".join(fields[:4]) + " 0 1")
    random.Random(7).shuffle(fens)
    return fens[:count]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("engine")
    parser.add_argument("nodes", type=int)
    parser.add_argument("count", type=int)
    parser.add_argument("--perf", metavar="OUT.data")
    parser.add_argument("--stat", action="store_true")
    parser.add_argument("--hash", type=int, default=64)
    parser.add_argument("--aggression", type=int, default=75)
    parser.add_argument("--book", default=str(DEFAULT_BOOK))
    parser.add_argument("--freq", default="3000")
    args = parser.parse_args()

    command = [args.engine]
    if args.perf:
        command = ["perf", "record", "-o", args.perf, "-F", args.freq, "--"] + command
    elif args.stat:
        events = "cycles,instructions,branches,branch-misses,L1-dcache-load-misses,cache-misses"
        command = ["perf", "stat", "-e", events, "--"] + command
    process = subprocess.Popen(
        command,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        text=True,
        bufsize=1,
    )

    def send(line):
        process.stdin.write(line + "\n")
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
    send(f"setoption name Aggression value {args.aggression}")
    send("isready")
    until("readyok")

    nps_values, depths, total_nodes, total_ms = [], [], 0, 0
    for fen in read_book(args.book, args.count):
        send("ucinewgame")
        send("isready")
        until("readyok")
        send(f"position fen {fen}")
        send(f"go nodes {args.nodes}")
        last = None
        for line in until("bestmove"):
            fields = line.split()
            if fields[:1] == ["info"] and "nps" in fields and "nodes" in fields:
                last = fields
        if last is None:
            continue
        nps_values.append(int(last[last.index("nps") + 1]))
        depths.append(int(last[last.index("depth") + 1]))
        total_nodes += int(last[last.index("nodes") + 1])
        total_ms += int(last[last.index("time") + 1])
    send("quit")
    try:
        process.wait(timeout=30)
    except subprocess.TimeoutExpired:
        process.kill()

    print(
        f"positions={len(nps_values)} median_nps={statistics.median(nps_values):.0f} "
        f"mean_depth={statistics.mean(depths):.1f} total_nodes={total_nodes} "
        f"total_ms={total_ms} agg_nps={1000 * total_nodes / max(total_ms, 1):.0f}"
    )


if __name__ == "__main__":
    main()
