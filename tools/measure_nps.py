#!/usr/bin/env python3
"""Measure fixed-node search throughput of one UCI configuration.

Every position in the suite is searched with ``go nodes N`` on one thread and
the engine's own final ``nps`` report is taken. The median over positions is
printed as JSON, so a candidate with a neural evaluator can be compared with
the handcrafted evaluator on the same host. Throughput is host-dependent and
only meaningful as a same-run ratio.
"""

from __future__ import annotations

import argparse
import json
import statistics
import subprocess
import sys
from pathlib import Path


def read_suite(path: Path) -> list[str]:
    fens = []
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        fields = line.split()
        if len(fields) < 4:
            raise ValueError(f"{path}: expected at least four FEN fields: {line}")
        fens.append(" ".join(fields[:4]) + " 0 1")
    if not fens:
        raise ValueError(f"{path}: no positions")
    return fens


class Engine:
    def __init__(self, path: Path, aggression: int, eval_file: Path | None) -> None:
        self.process = subprocess.Popen(
            [str(path)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1
        )
        self.send("uci")
        self.read_until("uciok")
        self.send("setoption name Threads value 1")
        self.send(f"setoption name Aggression value {aggression}")
        if eval_file is not None:
            self.send(f"setoption name EvalFile value {eval_file}")
            self.send("setoption name Use NNUE value true")
        self.send("isready")
        for line in self.read_until("readyok"):
            if line.startswith("info string") and ("rejected" in line or "requires" in line):
                raise RuntimeError(line)

    def send(self, command: str) -> None:
        assert self.process.stdin is not None
        self.process.stdin.write(command + "\n")
        self.process.stdin.flush()

    def read_until(self, token: str) -> list[str]:
        assert self.process.stdout is not None
        lines = []
        while True:
            line = self.process.stdout.readline()
            if not line:
                raise RuntimeError("engine closed its output")
            line = line.strip()
            lines.append(line)
            if line == token or line.startswith(token + " "):
                return lines

    def nps(self, fen: str, nodes: int) -> tuple[int, int]:
        self.send("ucinewgame")
        self.send("isready")
        self.read_until("readyok")
        self.send(f"position fen {fen}")
        self.send(f"go nodes {nodes}")
        nps = depth = None
        for line in self.read_until("bestmove"):
            fields = line.split()
            if fields[:1] == ["info"] and "nps" in fields:
                nps = int(fields[fields.index("nps") + 1])
                depth = int(fields[fields.index("depth") + 1])
        if nps is None or depth is None:
            raise RuntimeError("engine reported no nps")
        return nps, depth

    def close(self) -> None:
        try:
            self.send("quit")
        finally:
            self.process.wait(timeout=10)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--eval-file", type=Path)
    parser.add_argument("--aggression", type=int, default=75)
    parser.add_argument("--suite", type=Path, default=Path("tests/data/search-performance.epd"))
    parser.add_argument("--nodes", type=int, default=300_000)
    parser.add_argument("--repeats", type=int, default=3)
    args = parser.parse_args(argv)
    try:
        fens = read_suite(args.suite)
        engine = Engine(args.engine, args.aggression, args.eval_file)
        samples, depths = [], []
        try:
            for _ in range(args.repeats):
                for fen in fens:
                    nps, depth = engine.nps(fen, args.nodes)
                    samples.append(nps)
                    depths.append(depth)
        finally:
            engine.close()
    except (OSError, ValueError, RuntimeError) as error:
        print(f"measure_nps: {error}", file=sys.stderr)
        return 1
    print(json.dumps({
        "engine": str(args.engine), "eval_file": None if args.eval_file is None else str(args.eval_file),
        "aggression": args.aggression, "nodes": args.nodes, "positions": len(fens), "repeats": args.repeats,
        "median_nps": statistics.median(samples), "min_nps": min(samples), "max_nps": max(samples),
        "median_depth": statistics.median(depths),
    }, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
