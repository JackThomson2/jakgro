# A principal-variation bit in the transposition table

This report records the seventh round taken from the
[Elo opportunity audit](elo-opportunity-audit.md), its item T3. Every form
was rejected; the report exists so that none is tried again in the same form. The series
head was `0e38dd7`; every match below is against that head at Aggression 75
on both sides, 4096 paired games from the random-walk book, Hash at its
16 MiB default. Style ratios are the candidate's forcing-move and check rates
per move against the baseline's over the same games, with a bootstrap over the
2048 colour-reversed opening pairs.

## What the audit found

The search knew a node was on the principal variation only while its window
was open. A node that an earlier iteration had searched with an open window,
and that the present iteration reaches with a null window because the root
has since narrowed its bounds, was pruned by reverse futility and reduced
like any other null-window node. Stockfish keeps a bit in the table entry
(`is_pv`), sets `ttPv = PvNode || tte->is_pv()` at every node, skips reverse
futility and reduces late moves two plies less at such nodes, widens the
singular margin where a node that was on the variation is now searched with
a null window, marks a node that fails low under a parent on the variation,
and favours such entries in replacement. The engine's packed move field had
the one free bit this needs.

## The patch

The table entry gains a principal bit above the fifteen bits of its move. A
node is principal when its window is open or its entry says it was; the bit
is stored with every result the node writes, including the depth-zero
evaluation entries and the quiescence stores, which copy the entry's bit
forward. Reverse futility is skipped at principal nodes, late moves are
reduced one ply less there, unless the node is on the variation with a
stored upper bound at this depth that says it will fail low again, the
singular margin grows from two to three times the depth at a principal node
searched with a null window, and a node that fails low at depth four or more
under a principal parent is marked principal when it is stored. Replacement
is unchanged; that is the audit's item T1.

## Variants

| Form | Nodes at depth 13, Aggression 75 / 0 | 50 ms | Forcing / checks at 50 ms | 10+0.1 | Forcing / checks at 10+0.1 |
|---|---|---|---|---|---|
| full form as above | 22,283,597 (+5.1%) / 19,842,649 (−0.7%) | −5.4 [−11.7, +0.8] | 1.009 [0.991, 1.027] / 1.032 [0.985, 1.081] | −1.0 [−6.3, +4.2] | 1.006 [0.988, 1.025] / 1.016 [0.969, 1.066] |
| without the fail-low marking | 21,537,736 (+1.6%) / 20,871,510 (+4.4%) | +1.9 [−4.3, +8.1] | 1.004 [0.986, 1.022] / 1.018 [0.972, 1.066] | −3.0 [−8.3, +2.3] | 1.008 [0.990, 1.027] / 1.012 [0.964, 1.064] |
| bit stored and propagated, used only to skip reverse futility | 22,674,507 (+6.9%) / 19,713,212 (−1.4%) | +2.0 [−4.1, +8.2] | 0.976 [0.959, 0.992] / **0.930 [0.891, 0.970]** | not run | — |

None is kept. The full form is negative at 50 ms and null at 10+0.1; without
the fail-low marking it is null at both; and the form that uses the bit only
to skip reverse futility reads as much at 50 ms but checks seven percent
less often, since the nodes it now searches in full are the ones the
attacking profiles' checks used to settle by the margin. Stockfish's
accounting puts the whole family at a few Elo, and in this engine the
reductions and margins the bit relaxes are already milder than the ones it
was designed for: the late-move reduction keeps one ply on every quiet and
exempts the protected moves, and reverse futility stops at depth eight, so
the nodes the bit would protect are protected already. The audit's estimate
of two to eight Elo is not borne out; the device is recorded as measured so
that it is not rebuilt in this form. What remains of it is the replacement
preference for principal entries (the audit's T1), which this series did not
touch and which needs the longer-control harness.

## Fixtures that moved

None: no form was kept, so the suites are as the series head left them. For
the record, the full form moved four of them at 20,000 nodes
(`win-hanging-queen` 497 to 508 with its move, `storm-before-castling` at
Aggression 100 from the storm h7h5 to castling, `avoid-queen-trade-queen-c7`
and `queenside-pawn-thrust` at Aggression 0), which is the churn a device
that changes pruning at every formerly-principal node produces.
