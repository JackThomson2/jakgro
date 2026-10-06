#!/usr/bin/env python3
"""Per-move time and depth profile of Jakgro against its opponents.

usage: time_profile.py PGN [PGN ...] [--name PREFIX] [--step N]

Reads fastchess PGNs whose move comments carry ``{eval/depth seconds}`` and a
``TimeControl`` header of the form ``base+increment``. For every move number it
prints the mean seconds spent and completed depth of the engine whose name
starts with PREFIX (default ``Jakgro``) and of its opponents, together with the
mean fraction of the base time each side still had on its clock before the
move. Requires python-chess.
"""
import argparse
import re
from collections import defaultdict

import chess.pgn

COMMENT = re.compile(r"([+-]?\d+\.\d+|[+-]?M\d+)/(\d+) (\d+\.\d+)s")
TIME_CONTROL = re.compile(r"(\d+)\+(\d+(?:\.\d+)?)")


def mean(values):
    return sum(values) / len(values) if values else float("nan")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pgns", nargs="+")
    parser.add_argument("--name", default="Jakgro")
    parser.add_argument("--step", type=int, default=3)
    args = parser.parse_args()

    spend = {True: defaultdict(list), False: defaultdict(list)}
    depth = {True: defaultdict(list), False: defaultdict(list)}
    remaining = {True: defaultdict(list), False: defaultdict(list)}
    games = 0
    for path in args.pgns:
        with open(path) as handle:
            while True:
                game = chess.pgn.read_game(handle)
                if game is None:
                    break
                match = TIME_CONTROL.match(game.headers.get("TimeControl", ""))
                if not match:
                    continue
                base, increment = float(match.group(1)), float(match.group(2))
                white_ours = game.headers["White"].startswith(args.name)
                black_ours = game.headers["Black"].startswith(args.name)
                if not (white_ours or black_ours):
                    continue
                games += 1
                clocks = {True: base, False: base}
                node = game
                ply = 0
                while node.variations:
                    node = node.variation(0)
                    ply += 1
                    white_to_move = ply % 2 == 1
                    comment = COMMENT.search(node.comment)
                    if not comment:
                        continue
                    seconds = float(comment.group(3))
                    before = clocks[white_to_move]
                    clocks[white_to_move] = before - seconds + increment
                    move_number = (ply + 1) // 2
                    ours = (white_to_move and white_ours) or (not white_to_move and black_ours)
                    spend[ours][move_number].append(seconds)
                    depth[ours][move_number].append(int(comment.group(2)))
                    remaining[ours][move_number].append(before / base)

    print(f"games with {args.name}: {games}")
    print("move  n     ours_time  opp_time  ours_depth  opp_depth  ours_clock_before  opp_clock_before")
    for move_number in range(1, 201, args.step):
        if not spend[True][move_number]:
            continue
        print(
            f"{move_number:4d} {len(spend[True][move_number]):5d} "
            f"{mean(spend[True][move_number]):9.2f} {mean(spend[False][move_number]):9.2f} "
            f"{mean(depth[True][move_number]):10.1f} {mean(depth[False][move_number]):10.1f} "
            f"{mean(remaining[True][move_number]):17.2f} {mean(remaining[False][move_number]):16.2f}"
        )


if __name__ == "__main__":
    main()
