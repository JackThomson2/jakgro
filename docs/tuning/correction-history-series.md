# Correction history: the search teaching the evaluation inside a game

This report records the fourth change taken from the
[Elo opportunity audit](elo-opportunity-audit.md), its item Q1. The series
head was `85207e4`, which carries the late-move-reduction and ordering
patches of the [previous series](late-move-reduction-series.md); every match
below is against that head at Aggression 75 on both sides, 4096 paired games
from the random-walk book, Hash at its 16 MiB default.

## What the audit found

The network reads a position from its pieces and squares. It has no input for
the halfmove clock and none for the drawishness of a material class, and the
search had no device that learned, within one game, that a class of positions
keeps returning less than the network says: in the external gauntlet 94.5% of
the positions Jakgro scored at +2.5 or more with five pieces or fewer were
drawn (`syzygy.md`), and two thirds of the ones a Stockfish oracle judged
equal had eight pieces or fewer, past where tablebases stop.

A correction history is the standard answer in engines that followed
Stockfish's: tables keyed by features of the position record the difference
between what the search returned and what the evaluation said, and later
evaluations of positions sharing the key are moved by the record. Jakgro had
rejected four forms, all at 50 ms (`strength-series-three.md`,
`strength-series-four.md`, `search-selectivity-series.md`,
`search-store-razor-series.md`): +1.1 [−5.5, +7.7] for a pawn-structure key
applied to the pruning margins, −8.1 [−14.8, −1.5] for a pawn and material
count key applied to the stand-pat on the handcrafted evaluation, −6.5 and
−1.2 [−7.3, +4.9] for a pawn key applied to the main search's pruning
evaluation. The audit's reading of those forms, against the published ones:
the tables were created cold for every search, so at 50 ms they held little by
the time they were read; the keys were pawn bitboards and piece counts rather
than each side's non-pawn placement; the update accepted capture best moves,
whose material gain the evaluation never claimed to include; the learning
rate was several times below the public form; and the corrections were
applied to pruning margins rather than to the evaluation every consumer
reads.

## The patch

The move ordering, which the main searcher already carries from one move of
a game to the next and halves between moves, gains a correction history: a
table keyed by the pawn structure and one by each side's non-pawn material,
the king included, each entry indexed by the side to move as well, 16,384
slots per side and table. The keys multiply each bitboard by its own odd
constant before a 64-bit mix.

A node that finishes its move loop outside check, in the normal search mode
and not excluding a move, with a result below the mate and tablebase band,
records `(result − corrected evaluation) × depth / 8`, bounded at ±256, into
its three entries when the best move is not a capture or promotion and the
bound points the same way as the difference: a result below the evaluation
that is also below beta, or above the evaluation and above the original
alpha. The update is `entry += step − entry × |step| / 1024`, which keeps an
entry inside ±1024 and lets a residual of the other sign overrule an old
one. Every static evaluation the search prunes, compares and stands pat on,
in the main search and in quiescence, is the raw value plus
`(24 × pawn + 16 × (white material + black material)) / 1024`, at most 56
centipawns; the table keeps the raw value, so a stored evaluation is
corrected once when it is read, and the quiescence the razoring hands its
evaluation to receives the raw value and corrects it itself. The slots are
computed once per node and shared by the read and the update, which keeps
the cost at about 3% of throughput (2.44M to 2.36M nodes per second over the
forty-position book at 600,000 nodes each).

## Matches

| Form | Nodes at depth 13, Aggression 75 / 0 | 50 ms | 10+0.1 | Forcing / checks at 10+0.1 |
|---|---|---|---|---|
| keys as a plain exclusive-or of the bitboards | 23,112,792 / 19,873,123 | +1.8 [−4.2, +7.8] | — | — |
| **keys with per-bitboard multipliers (kept)** | 21,205,941 / 19,985,482 | −3.8 [−10.0, +2.4] | **+10.3 [+5.4, +15.2]** | 1.024 [1.006, 1.042] / 1.060 [1.010, 1.108] |

The first key form came out of the unit test that pins the key properties:
a mirrored pair of pawn moves (a2a3 with a7a6) left the exclusive-or of the
two pawn boards unchanged, so symmetric structures shared a slot, and pieces
exchanging squares did the same in the material key. Multiplying each board
by its own constant before the mix removes the linearity. Its 50 ms match is
kept in the table as the measurement of the idea at that control.

The kept form is null at 50 ms and worth ten Elo at 10+0.1, with the forcing
and check rates up by two and six percent. The split is the one the audit
predicted and the one the public record of this device shows: at depth 7 a
search makes few updates before it must move, so the tables hold little by
the time they are read, while at depth 17 the same tables have learned the
pawn structure and material of the game at hand. Both 50 ms readings (+1.8
and −3.8) are inside the noise of that control; neither is evidence of a
cost there. The device is kept on the 10+0.1 match.

## Fixtures that moved

Four regression records move with their trees. `win-hanging-queen` 499 to
497, `contain-lone-rook` −529 to −527 and `starting-style` 24 to 16 keep
their moves; `developed-open-game` plays f1c4 at 30 rather than d2d4 at 37
at 20,000 nodes, the move the head itself plays from 40,000 nodes. In the
standard-acceptance suite the default profile plays the objective f1c4 in
`standard-central-space` at 20,000 nodes, which the head plays from 40,000,
so bm75 accepts it beside d2d4; and the objective profile plays d2a5 in
`standard-avoid-equal-queen-trade`, another move tied near zero in a bare
queen ending, so bm0 accepts it beside d2b2 and d2h6.

`rook-f3-investment`, in the personality and sacrifice-gate suites, rested
on a 20,000-node window. Probed across budgets, the head's objective profile
plays the f8f3 investment at 10,000 nodes and again from 60,000 to 160,000,
declining it for d7e5 only between 20,000 and 40,000; with the correction it
plays f8f3 from 10,000 to 80,000 and declines it only from 100,000, while
Aggression 100 plays f8f3 at every budget on both binaries. The record moves
to 160,000 nodes, where the objective profile plays e7f6 and Aggression 100
f8f3 at every budget probed from 100,000 to 300,000, and the contract's hash
of the personality suite follows. The null-move and UCI suites passed as
they were.
