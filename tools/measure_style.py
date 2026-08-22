#!/usr/bin/env python3
"""Measure deterministic fixed-node style choices through the UCI interface."""

from __future__ import annotations

import argparse
import csv
import queue
import subprocess
import sys
import threading
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class Fixture:
    identifier: str
    fen: str
    nodes: int
    expected: dict[int, str]


@dataclass(frozen=True)
class Observation:
    bestmove: str
    score: str
    depth: int
    nodes: int


def parse_suite(path: Path) -> list[Fixture]:
    fixtures: list[Fixture] = []
    identifiers: set[str] = set()
    for line_number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        fields = [field.strip() for field in line.split(";")]
        fen = fields[0]
        values: dict[str, str] = {}
        expected: dict[int, str] = {}
        for field in fields[1:]:
            if not field:
                continue
            try:
                key, value = field.split(maxsplit=1)
            except ValueError as error:
                raise ValueError(f"{path}:{line_number}: malformed field {field!r}") from error
            if key.startswith("bm") and key[2:].isdigit():
                profile = int(key[2:])
                if profile in expected:
                    raise ValueError(f"{path}:{line_number}: duplicate field {key!r}")
                expected[profile] = value
            elif key in {"id", "nodes"}:
                if key in values:
                    raise ValueError(f"{path}:{line_number}: duplicate field {key!r}")
                values[key] = value
            else:
                raise ValueError(f"{path}:{line_number}: unsupported field {key!r}")
        identifier = values.get("id")
        if not identifier:
            raise ValueError(f"{path}:{line_number}: missing id")
        if identifier in identifiers:
            raise ValueError(f"{path}:{line_number}: duplicate id {identifier!r}")
        identifiers.add(identifier)
        try:
            nodes = int(values["nodes"])
        except (KeyError, ValueError) as error:
            raise ValueError(f"{path}:{line_number}: invalid node budget") from error
        if nodes <= 0 or set(expected) != {0, 100}:
            raise ValueError(
                f"{path}:{line_number}: positive nodes plus bm0 and bm100 are required"
            )
        fixtures.append(Fixture(identifier, fen, nodes, expected))
    if not fixtures:
        raise ValueError(f"{path}: no fixtures")
    return fixtures


def parse_profiles(value: str) -> list[int]:
    try:
        profiles = [int(item.strip()) for item in value.split(",")]
    except ValueError as error:
        raise argparse.ArgumentTypeError("profiles must be comma-separated integers") from error
    if not profiles or any(profile < 0 or profile > 100 for profile in profiles):
        raise argparse.ArgumentTypeError("profiles must be between 0 and 100")
    if len(set(profiles)) != len(profiles):
        raise argparse.ArgumentTypeError("profiles must be unique")
    return profiles


def parse_search_info(line: str) -> tuple[str, int, int] | None:
    if not line.startswith("info "):
        return None
    tokens = line.split()
    try:
        depth = int(tokens[tokens.index("depth") + 1])
        score_index = tokens.index("score")
        score = f"{tokens[score_index + 1]} {tokens[score_index + 2]}"
        nodes = int(tokens[tokens.index("nodes") + 1])
    except (ValueError, IndexError):
        return None
    return score, depth, nodes


class UciEngine:
    def __init__(self, executable: Path, timeout: float) -> None:
        self.timeout = timeout
        self.process = subprocess.Popen(
            [str(executable)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            text=True,
            bufsize=1,
        )
        assert self.process.stdin is not None
        assert self.process.stdout is not None
        self.stdin = self.process.stdin
        self.lines: queue.Queue[str | None] = queue.Queue()
        self.reader = threading.Thread(target=self._read_output, daemon=True)
        self.reader.start()
        try:
            self.send("uci")
            handshake = self.read_until(lambda line: line == "uciok")
            if not any(line.startswith("option name Aggression type spin ") for line in handshake):
                raise RuntimeError("engine did not advertise the Aggression UCI option")
            self.send("debug on")
            self.send("isready")
            self.read_until(lambda line: line == "readyok")
        except BaseException:
            self.close()
            raise

    def _read_output(self) -> None:
        try:
            for line in self.process.stdout:
                self.lines.put(line.rstrip("\r\n"))
        finally:
            self.lines.put(None)

    def send(self, command: str) -> None:
        self.stdin.write(f"{command}\n")
        self.stdin.flush()

    def read_until(self, predicate) -> list[str]:
        output: list[str] = []
        while True:
            try:
                line = self.lines.get(timeout=self.timeout)
            except queue.Empty as error:
                raise TimeoutError("timed out waiting for UCI output") from error
            if line is None:
                raise RuntimeError(f"engine exited with status {self.process.poll()}")
            output.append(line)
            if predicate(line):
                return output

    def measure(self, fixture: Fixture, aggression: int) -> Observation:
        self.send(f"setoption name Aggression value {aggression}")
        self.send("ucinewgame")
        self.send("isready")
        self.read_until(lambda line: line == "readyok")
        self.send(f"position fen {fixture.fen}")
        self.send("isready")
        position_output = self.read_until(lambda line: line == "readyok")
        if any(line.startswith("info string position rejected:") for line in position_output):
            raise RuntimeError(f"{fixture.identifier}: engine rejected fixture FEN")
        self.send(f"go nodes {fixture.nodes}")
        output = self.read_until(lambda line: line.startswith("bestmove "))
        bestmove_fields = output[-1].split()
        parsed_info = next(
            (
                parsed
                for line in reversed(output[:-1])
                if (parsed := parse_search_info(line)) is not None
            ),
            None,
        )
        if len(bestmove_fields) < 2 or bestmove_fields[1] == "0000" or parsed_info is None:
            raise RuntimeError(f"{fixture.identifier}: search returned no measured result")
        score, depth, nodes = parsed_info
        return Observation(bestmove_fields[1], score, depth, nodes)

    def close(self) -> None:
        if self.process.poll() is None:
            try:
                self.send("quit")
                self.process.wait(timeout=self.timeout)
            except (BrokenPipeError, subprocess.TimeoutExpired):
                self.process.kill()
                self.process.wait()

    def __enter__(self) -> UciEngine:
        return self

    def __exit__(self, *_args) -> None:
        self.close()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine", type=Path, required=True, help="UCI engine executable")
    parser.add_argument(
        "--suite",
        type=Path,
        default=Path("tests/data/personality.epd"),
        help="personality EPD suite",
    )
    parser.add_argument("--profiles", type=parse_profiles, default=parse_profiles("0,100"))
    parser.add_argument("--timeout", type=float, default=10.0, help="seconds per UCI response")
    parser.add_argument("--check", action="store_true", help="fail on expected-move mismatch")
    args = parser.parse_args()
    if args.timeout <= 0:
        parser.error("--timeout must be positive")

    try:
        fixtures = parse_suite(args.suite)
        rows: list[list[object]] = []
        failures = 0
        with UciEngine(args.engine, args.timeout) as engine:
            for fixture in fixtures:
                for profile in args.profiles:
                    observation = engine.measure(fixture, profile)
                    expected = fixture.expected.get(profile, "")
                    mismatch = bool(expected) and observation.bestmove != expected
                    missing = args.check and not expected
                    passed = not mismatch and not missing
                    failures += int(mismatch or missing)
                    status = (
                        "unrated"
                        if not expected and not args.check
                        else ("pass" if passed else "FAIL")
                    )
                    rows.append(
                        [
                            fixture.identifier,
                            profile,
                            observation.bestmove,
                            expected,
                            observation.score,
                            observation.depth,
                            observation.nodes,
                            status,
                        ]
                    )
    except (OSError, RuntimeError, TimeoutError, ValueError) as error:
        print(f"measure_style: {error}", file=sys.stderr)
        return 2

    writer = csv.writer(sys.stdout, lineterminator="\n")
    writer.writerow(["id", "aggression", "bestmove", "expected", "score", "depth", "nodes", "status"])
    writer.writerows(rows)
    print(f"measured {len(rows)} searches; mismatches={failures}", file=sys.stderr)
    return 1 if args.check and failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
