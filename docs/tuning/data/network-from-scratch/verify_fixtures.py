#!/usr/bin/env python3
"""Sort mined positions into motifs and verify them across node budgets.

usage: verify_fixtures.py ENGINE MOTIF OUT.jsonl MINED.jsonl [...] [--max N]

MOTIF is one of sacrifice, simplification, pawn-storm, initiative. A
candidate holds when Aggression 0 and Aggression 100 each choose the same
move at 20k, 40k, 60k and 100k nodes and the two moves differ; its objective
loss is Aggression 0's score minus Aggression 0's score restricted to the
Aggression 100 move, both at 20k nodes and at 100k nodes.
"""
import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[4] / "tools"))
import chess  # noqa: E402
import measure_style  # noqa: E402

BUDGETS = (20000, 40000, 60000, 100000)


def motif_of(record: dict) -> set[str]:
    c0, c100 = record["c0"], record["c100"]
    board = chess.Board(record["fen"])
    motifs = set()
    if c100["capture"] and c100["investment"] >= 2 and not (c0["capture"] and c0["investment"] >= 2):
        motifs.add("sacrifice")
    if c0["queen_trade"] and not c100["queen_trade"] and not c100["capture"]:
        motifs.add("simplification")
    if (c100["piece"] == "p" and c100["pawn_toward_king"] and not c100["capture"]
            and not (c0["piece"] == "p" and c0["pawn_toward_king"])):
        # A storm needs the enemy king on a flank, castled or not.
        enemy_king = board.king(not board.turn)
        if chess.square_file(enemy_king) in (0, 1, 2, 5, 6, 7):
            motifs.add("pawn-storm")
    if (c100["piece"] == "p" and not c100["capture"] and not c100["pawn_toward_king"]
            and c0["piece"] != "p"):
        motifs.add("initiative")
    return motifs


def score_cp(score: str) -> int | None:
    kind, value = score.split()
    return int(value) if kind == "cp" else None


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("engine")
    parser.add_argument("motif")
    parser.add_argument("out")
    parser.add_argument("mined", nargs="+")
    parser.add_argument("--max", type=int, default=40)
    args = parser.parse_args()
    records = []
    for path in args.mined:
        with open(path) as source:
            records.extend(json.loads(line) for line in source if line.strip())
    chosen = records if args.motif.startswith("any") else [r for r in records if args.motif in motif_of(r)]
    print(f"{len(chosen)} {args.motif} candidates of {len(records)}", file=sys.stderr)
    engine = measure_style.UciEngine(Path(args.engine), timeout=120.0)
    held = 0
    try:
        with open(args.out, "a") as out:
            for record in chosen[: args.max]:
                fen = record["fen"]
                moves = {0: [], 100: []}
                scores = {0: [], 100: []}
                for profile in (0, 100):
                    for budget in BUDGETS:
                        obs = engine.measure(measure_style.Fixture("v", "v", fen, budget, {}), profile)
                        moves[profile].append(obs.bestmove)
                        scores[profile].append(obs.score)
                stable = (len(set(moves[0])) == 1 and len(set(moves[100])) == 1
                          and moves[0][0] != moves[100][0])
                result = {"fen": fen, "motif": args.motif, "moves": moves, "scores": scores, "stable": stable,
                          "c0": record["c0"], "c100": record["c100"]}
                if stable:
                    alt = moves[100][0]
                    losses = []
                    for budget in (20000, 100000):
                        fixture = measure_style.Fixture("v", "v", fen, budget, {})
                        free = engine.measure(fixture, 0)
                        restricted = engine.measure(fixture, 0, root_moves=frozenset([alt]))
                        a, b = score_cp(free.score), score_cp(restricted.score)
                        losses.append(None if a is None or b is None else a - b)
                    result["objective_loss"] = losses
                    held += 1
                out.write(json.dumps(result) + "\n")
                out.flush()
                print(f"{'HOLD' if stable else 'drop'} {fen} A0 {moves[0]} A100 {moves[100]} loss {result.get('objective_loss')}", flush=True)
    finally:
        engine.close()
    print(f"{held} held", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
