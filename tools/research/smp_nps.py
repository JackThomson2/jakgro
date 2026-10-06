#!/usr/bin/env python3
"""Throughput and completed depth at a fixed movetime for several thread counts.

usage: smp_nps.py ENGINE MOVETIME_MS COUNT THREADS... [--hash MIB] [--book EPD]

Each thread count starts a fresh engine process; the same COUNT positions
(fixed shuffle, seed 11) are searched with ``go movetime``. Prints the median
NPS, the mean completed depth and the per-position depths per thread count.
Depth at a fixed time is a weak proxy for lazy-SMP strength; the throughput
column is the reliable one.
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
    random.Random(11).shuffle(fens)
    return fens[:count]


def measure(engine, fens, threads, movetime, hash_mib):
    process = subprocess.Popen(
        [engine],
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
    send(f"setoption name Hash value {hash_mib}")
    send(f"setoption name Threads value {threads}")
    send("isready")
    until("readyok")
    nps_values, depths = [], []
    for fen in fens:
        send("ucinewgame")
        send("isready")
        until("readyok")
        send(f"position fen {fen}")
        send(f"go movetime {movetime}")
        last = None
        for line in until("bestmove"):
            fields = line.split()
            if fields[:1] == ["info"] and "nps" in fields and "depth" in fields:
                last = fields
        if last is None:
            continue
        nps_values.append(int(last[last.index("nps") + 1]))
        depths.append(int(last[last.index("depth") + 1]))
    send("quit")
    try:
        process.wait(timeout=30)
    except subprocess.TimeoutExpired:
        process.kill()
    return nps_values, depths


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("engine")
    parser.add_argument("movetime", type=int)
    parser.add_argument("count", type=int)
    parser.add_argument("threads", type=int, nargs="+")
    parser.add_argument("--hash", type=int, default=256)
    parser.add_argument("--book", default=str(DEFAULT_BOOK))
    args = parser.parse_args()

    fens = read_book(args.book, args.count)
    for threads in args.threads:
        nps_values, depths = measure(args.engine, fens, threads, args.movetime, args.hash)
        print(
            f"threads={threads} median_nps={statistics.median(nps_values):.0f} "
            f"mean_depth={statistics.mean(depths):.2f} depths={depths}",
            flush=True,
        )


if __name__ == "__main__":
    main()
