#!/usr/bin/env python3
"""Generate a labelled NNUE training corpus from deterministic engine self-play.

The `selfplay` arbiter plays fixed-node games between two aggression profiles
of one engine from seeded random-ply openings, and `tune extract` turns the
annotated PGN into `FEN;white-outcome;white-score-cp` rows: every position not
in check whose played move is neither a capture nor a promotion, labelled by
the game result and by the score the mover's search reported. Fixed nodes,
one thread and a fixed seed make the PGN, and so the corpus, reproducible.

The output is gzip text ready for `train_nnue.py prepare`, beside a manifest
binding the binaries, suite and settings by hash.
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def run(command: list[str], log: Path) -> None:
    with log.open("ab") as stream:
        stream.write((" ".join(command) + "\n").encode())
        stream.flush()
        result = subprocess.run(command, stdout=stream, stderr=subprocess.STDOUT, check=False)
    if result.returncode != 0:
        raise RuntimeError(f"{command[0]} exited {result.returncode}; see {log}")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine", type=Path, required=True, help="UCI engine used on both sides")
    parser.add_argument("--runner", type=Path, required=True, help="selfplay arbiter")
    parser.add_argument("--tune", type=Path, required=True, help="tuning-feature `tune` binary")
    parser.add_argument("--openings", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True, help="gzip corpus path")
    parser.add_argument("--games-per-seed", type=int, default=4096)
    parser.add_argument("--seeds", type=int, nargs="+", default=[1])
    parser.add_argument("--nodes", type=int, default=50_000)
    parser.add_argument("--random-plies", type=int, default=8)
    parser.add_argument("--candidate-aggression", type=int, default=75)
    parser.add_argument("--baseline-aggression", type=int, default=0)
    parser.add_argument("--concurrency", type=int, default=8)
    parser.add_argument("--skip-plies", type=int, default=8)
    args = parser.parse_args(argv)
    output = args.output.resolve()
    if output.exists():
        print(f"generate_nnue_corpus: {output} already exists", file=sys.stderr)
        return 1
    output.parent.mkdir(parents=True, exist_ok=True)
    log = output.with_suffix(".log")
    log.unlink(missing_ok=True)
    try:
        with tempfile.TemporaryDirectory(prefix=".nnue-corpus-", dir=output.parent) as temporary:
            work = Path(temporary)
            pgns = []
            for seed in args.seeds:
                pgn = work / f"seed-{seed}.pgn"
                run([
                    str(args.runner), "--engine", str(args.engine),
                    "--candidate-aggression", str(args.candidate_aggression),
                    "--baseline-aggression", str(args.baseline_aggression),
                    "--games", str(args.games_per_seed), "--nodes", str(args.nodes),
                    "--openings", str(args.openings), "--random-plies", str(args.random_plies),
                    "--seed", str(seed), "--concurrency", str(args.concurrency),
                    "--event", f"NNUE corpus seed {seed}", "--pgn", str(pgn),
                ], log)
                pgns.append(pgn)
            positions = work / "positions.txt"
            run([str(args.tune), "extract", "--pgn", *map(str, pgns), "--out", str(positions),
                 "--skip-plies", str(args.skip_plies)], log)
            rows = sum(1 for _ in positions.open(encoding="utf-8"))
            staged = work / "corpus.txt.gz"
            with positions.open("rb") as source, gzip.GzipFile(staged, "wb", mtime=0) as target:
                shutil.copyfileobj(source, target)
            manifest = {
                "schema_version": 1, "rows": rows, "games": args.games_per_seed * len(args.seeds),
                "settings": {"games_per_seed": args.games_per_seed, "seeds": args.seeds, "nodes": args.nodes,
                             "random_plies": args.random_plies, "skip_plies": args.skip_plies,
                             "candidate_aggression": args.candidate_aggression,
                             "baseline_aggression": args.baseline_aggression},
                "inputs": {name: {"path": str(path), "sha256": sha256(path)} for name, path in
                           (("engine", args.engine), ("runner", args.runner), ("tune", args.tune),
                            ("openings", args.openings))},
                "pgn_sha256": [sha256(pgn) for pgn in pgns],
                "corpus_sha256": sha256(staged),
            }
            output.with_suffix(".manifest.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
            staged.rename(output)
    except (OSError, RuntimeError) as error:
        print(f"generate_nnue_corpus: {error}", file=sys.stderr)
        return 1
    print(json.dumps({"output": str(output), "rows": rows}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
