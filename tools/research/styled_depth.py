#!/usr/bin/env python3
"""Personality (styled-root) telemetry as a function of search depth.

usage: styled_depth.py ENGINE COUNT DEPTH... [--aggression N] [--hash MIB] [--book EPD]

Runs ``go depth D`` with ``debug on`` over COUNT positions (fixed shuffle,
seed 5) for every depth given and sums the ``info string personality
nodes= attempts= completed= exhausted= selections=`` counters the engine prints
at the end of each search. The counters are cumulative over the iterations of
one search, so differencing consecutive depths gives the per-depth rates:
a completed-verification or selection count that stops growing shows the depth
beyond which the styled root no longer decides anything.
"""
import argparse
import random
import re
import subprocess
from pathlib import Path

DEFAULT_BOOK = (
    Path(__file__).resolve().parents[2]
    / "docs"
    / "tuning"
    / "data"
    / "selective-search-confirmation.epd"
)
PERSONALITY = re.compile(
    r"personality nodes=(\d+) attempts=(\d+) completed=(\d+) exhausted=(\d+) selections=(\d+)"
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
    random.Random(5).shuffle(fens)
    return fens[:count]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("engine")
    parser.add_argument("count", type=int)
    parser.add_argument("depths", type=int, nargs="+")
    parser.add_argument("--aggression", type=int, default=75)
    parser.add_argument("--hash", type=int, default=16)
    parser.add_argument("--book", default=str(DEFAULT_BOOK))
    args = parser.parse_args()

    fens = read_book(args.book, args.count)
    process = subprocess.Popen(
        [args.engine],
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
    send("debug on")
    send("isready")
    until("readyok")
    print("depth  positions  nodes_total  pers_nodes  pers_share  attempts  completed  exhausted  selections")
    for depth in args.depths:
        totals = {"nodes": 0, "pers": 0, "attempts": 0, "completed": 0, "exhausted": 0, "selections": 0}
        for fen in fens:
            send("ucinewgame")
            send("isready")
            until("readyok")
            send(f"position fen {fen}")
            send(f"go depth {depth}")
            last_nodes = 0
            for line in until("bestmove"):
                match = PERSONALITY.search(line)
                if match:
                    for key, value in zip(("pers", "attempts", "completed", "exhausted", "selections"), match.groups()):
                        totals[key] += int(value)
                fields = line.split()
                if fields[:1] == ["info"] and "nodes" in fields:
                    last_nodes = int(fields[fields.index("nodes") + 1])
            totals["nodes"] += last_nodes
        share = totals["pers"] / max(1, totals["nodes"])
        print(
            f"{depth:5d} {len(fens):10d} {totals['nodes']:12d} {totals['pers']:11d} {share:10.3%} "
            f"{totals['attempts']:9d} {totals['completed']:10d} {totals['exhausted']:10d} {totals['selections']:11d}",
            flush=True,
        )
    send("quit")
    try:
        process.wait(timeout=30)
    except subprocess.TimeoutExpired:
        process.kill()


if __name__ == "__main__":
    main()
