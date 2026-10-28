#!/usr/bin/env python3
"""Mine profile-separating positions for the style fixtures.

usage: mine_fixtures.py ENGINE OUT.jsonl CORPUS [CORPUS...] --slice K/N --limit M

Samples quiet-ish middlegame positions (|label score| <= 120, >= 18 pieces,
side to move not in check) from fen;outcome;score corpus files, searches each
at Aggression 0 and 100 with 20,000 nodes and fresh tables, and writes every
position where the two profiles choose different moves, with the move
classification needed to assign a motif.
"""
import argparse
import json
import random
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[4] / "tools"))
import chess  # noqa: E402
import measure_style  # noqa: E402

VALUE = {chess.PAWN: 1, chess.KNIGHT: 3, chess.BISHOP: 3, chess.ROOK: 5, chess.QUEEN: 9, chess.KING: 0}


def see(board: chess.Board, move: chess.Move) -> int:
    """Swap-list exchange value of a move, in pawns, ignoring pins."""
    work = board.copy(stack=False)
    target = move.to_square
    victim = work.piece_at(target)
    gains = [VALUE[victim.piece_type] if victim else 0]
    mover = work.piece_at(move.from_square)
    on_square = VALUE[mover.piece_type] if mover else 0
    if move.promotion:
        gains[0] += VALUE[move.promotion] - 1
        on_square = VALUE[move.promotion]
    work.remove_piece_at(move.from_square)
    work.set_piece_at(target, chess.Piece(mover.piece_type if not move.promotion else move.promotion, board.turn))
    side = not board.turn
    while True:
        attackers = [sq for sq in work.attackers(side, target)]
        if not attackers:
            break
        sq = min(attackers, key=lambda s: VALUE[work.piece_at(s).piece_type] or 100)
        piece = work.piece_at(sq)
        if piece.piece_type == chess.KING and work.attackers(not side, target):
            break
        gains.append(on_square - gains[-1])
        on_square = VALUE[piece.piece_type] or 100
        work.remove_piece_at(sq)
        work.set_piece_at(target, piece)
        side = not side
    while len(gains) > 1:
        last = gains.pop()
        gains[-1] = -max(-gains[-1], last)
    return gains[0]


def classify(board: chess.Board, uci: str) -> dict:
    move = chess.Move.from_uci(uci)
    piece = board.piece_at(move.from_square)
    victim = board.piece_at(move.to_square)
    info = {
        "move": uci,
        "piece": chess.piece_symbol(piece.piece_type) if piece else "?",
        "capture": victim is not None or board.is_en_passant(move),
        "check": board.gives_check(move),
        "castle": board.is_castling(move),
    }
    if victim is not None and piece is not None:
        info["investment"] = VALUE[piece.piece_type] - VALUE[victim.piece_type]
        info["queen_trade"] = piece.piece_type == chess.QUEEN and victim.piece_type == chess.QUEEN
    else:
        info["investment"] = 0
        info["queen_trade"] = False
    # Hanging the moved piece: attacked by a cheaper piece or undefended.
    after = board.copy()
    after.push(move)
    attackers = after.attackers(after.turn, move.to_square)
    defenders = after.attackers(not after.turn, move.to_square)
    moved_value = VALUE[piece.piece_type] if piece else 0
    cheaper = any(VALUE[after.piece_at(sq).piece_type] < moved_value for sq in attackers)
    info["offers"] = bool(attackers) and (cheaper or not defenders) and moved_value >= 3
    info["see"] = see(board, move)
    if piece is not None and piece.piece_type == chess.PAWN:
        enemy_king = board.king(not board.turn)
        info["pawn_toward_king"] = abs(chess.square_file(move.to_square) - chess.square_file(enemy_king)) <= 1
        info["pawn_advance"] = abs(chess.square_rank(move.to_square) - chess.square_rank(move.from_square))
    else:
        info["pawn_toward_king"] = False
        info["pawn_advance"] = 0
    return info


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("engine")
    parser.add_argument("out")
    parser.add_argument("corpus", nargs="+")
    parser.add_argument("--slice", default="0/1")
    parser.add_argument("--limit", type=int, default=2000)
    parser.add_argument("--nodes", type=int, default=20000)
    parser.add_argument("--seed", type=int, default=29)
    parser.add_argument("--offset", type=int, default=0)
    args = parser.parse_args()
    k, n = (int(x) for x in args.slice.split("/"))
    candidates = []
    for path in args.corpus:
        with open(path) as source:
            for index, line in enumerate(source):
                if index % 7:
                    continue
                fen, _outcome, score = line.strip().split(";")
                if abs(int(score)) > 120:
                    continue
                board = chess.Board(fen)
                if len(board.piece_map()) < 18 or board.is_check():
                    continue
                candidates.append(fen)
    random.Random(args.seed).shuffle(candidates)
    seen = set()
    unique = []
    for fen in candidates:
        key = " ".join(fen.split()[:4])
        if key not in seen:
            seen.add(key)
            unique.append(fen)
    mine = unique[k::n][args.offset: args.offset + args.limit]
    print(f"slice {k}/{n}: {len(mine)} positions of {len(unique)}", file=sys.stderr)
    engine = measure_style.UciEngine(Path(args.engine), timeout=120.0)
    found = 0
    with open(args.out, "a") as out:
        try:
            for count, fen in enumerate(mine):
                fixture = measure_style.Fixture("m", "m", fen, args.nodes, {})
                a0 = engine.measure(fixture, 0)
                a100 = engine.measure(fixture, 100)
                selections = a100.personality.get("selections", 0) if a100.personality else 0
                if a0.bestmove == a100.bestmove:
                    continue
                board = chess.Board(fen)
                record = {
                    "fen": fen,
                    "a0": {"move": a0.bestmove, "score": a0.score, "depth": a0.depth},
                    "a100": {"move": a100.bestmove, "score": a100.score, "depth": a100.depth, "selections": selections},
                    "c0": classify(board, a0.bestmove),
                    "c100": classify(board, a100.bestmove),
                }
                out.write(json.dumps(record) + "\n")
                out.flush()
                found += 1
                if count % 100 == 0:
                    print(f"{count} searched, {found} separating", file=sys.stderr)
        finally:
            engine.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
