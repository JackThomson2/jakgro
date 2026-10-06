# Late-move reductions: coverage, floor and the forcing-move exemptions

This report records the first search change taken from the
[Elo opportunity audit](elo-opportunity-audit.md), its item S1. The series
head was `e706387`; every match below is against that head at Aggression 75 on
both sides, 4096 paired games from the random-walk book, with the Hash at its
16 MiB default.

## What the audit found

At depth 14 over forty book positions the search reduced 20% of the moves it
considered and re-searched **0.4%** of the reductions (section 1.2 of the
audit). Stockfish re-searches a double-digit share of its reduced moves; a
re-search rate that low is the signature of reductions too mild and too
narrowly applied. The late-move reduction applied only from the third move at
a child depth of at least three (the sixth at depth two), never to a quiet
check, a castling move or a move into the enemy king's zone, and never below
two remaining plies. Stockfish reduces from the second move at depth two down
to one ply with none of those exemptions.

Two of the exemptions had been measured alone at 50 ms: the one-ply floor
twice (+5.6 [−0.6, +11.8] and +1.4 [−4.7, +7.4]) and king-zone moves reduced
like other quiets (+1.4 [−4.7, +7.6]), each too small to keep on its own
(`search-table-clock-series.md`). The audit's argument was that reductions
matter more at the depth of rated play, 17–20, than at the depth 7 of the 50 ms
test regime, and that the exemptions should be measured together.

## Variants

Four variants were screened by node counts at depth 13 over the forty-position
book, then matched.

| Variant | Change | Nodes at Aggression 75 | Aggression 0 |
|---|---|---:|---:|
| head | reductions from move 3 at depth ≥ 3, floor 2, checks/king-zone/castling exempt | 26,995,867 | 24,857,646 |
| B | from move 2 at depth ≥ 2, floor 1 | 26,491,790 (−1.9%) | 22,116,926 (−11.0%) |
| C | B, and checks and king-zone quiets reduced like other quiets | 23,831,363 (−11.7%) | 20,323,165 (−18.2%) |
| C′ | B, and checks and king-zone quiets reduced **one ply less** than other quiets | 22,796,248 (−15.6%) | 19,817,581 (−20.3%) |
| C3 | B, and king-zone quiets reduced one ply less; checks stay exempt | not screened | not screened |

C′ searches fewer nodes than C although it reduces less: a reduction one ply
shorter on a forcing move saves the re-search that a full reduction provokes
more often than it costs.

## Matches

Style ratios are the candidate's forcing-move and check rates per move
against the baseline's over the same games, with a bootstrap over the 2048
colour-reversed opening pairs (`artifacts/research/tools/style_bootstrap.py`);
1.000 is no change.

| Variant | 50 ms | 10+0.1 | Forcing / checks at 50 ms | Forcing / checks at 10+0.1 |
|---|---|---|---|---|
| B | +0.3 [−5.9, +6.4] | — | 0.995 [0.976, 1.013] / 0.981 [0.935, 1.028] | — |
| C | +8.1 [+2.0, +14.3] | +8.5 [+3.5, +13.5] | 0.989 [0.971, 1.008] / 0.963 [0.919, 1.010] | 0.976 [0.959, 0.993] / 0.921 [0.879, 0.965] |
| **C′** | **+9.3 [+3.3, +15.4]** | **+11.2 [+6.1, +16.3]** | 1.009 [0.992, 1.027] / 1.014 [0.969, 1.058] | 0.988 [0.970, 1.006] / 0.956 [0.911, 1.003] |
| C3 | +4.6 [−1.5, +10.6] | +0.2 [−5.0, +5.3] | 1.006 [0.988, 1.023] / 1.007 [0.962, 1.053] | 0.973 [0.956, 0.990] / 0.927 [0.883, 0.971] |

The coverage and floor change alone (B) is worth nothing measurable at 50 ms,
as its two earlier matches said. The gain is in reducing the forcing quiets
the search used to exempt, and reducing them one ply less than other quiets
(C′) beats reducing them fully (C) at both controls. C3, which keeps the
check exemption and reduces only king-zone quiets, gives up half of the gain
at 50 ms and all of it at 10+0.1: the check reduction is where the gain is at
depth.

On style, C′ leaves the forcing and check rates unchanged at 50 ms. At 10+0.1
its check rate reads about four percent below the head's with an interval
that reaches 1.0, but C3, which reduces checks less than C′ and should
therefore check at least as often, reads seven percent below with the same
head, and C reads eight. The same-profile check ratio at this control
carries more game-path variance than the differences between the variants,
so C′'s reading is not evidence of a style loss; its forcing rate is within
two percent at both controls. The Aggression 75 against 0 ratios, which are
what the style contract gates, were not re-measured here and are the next
check if the rate keeps reading low.

C′ is kept. The gain grows with the time control, as the audit expected of a
reduction change measured until now only at depth 7.

## The patch

`late_move_reduction` now applies from the second move at child depth two, so
the separate shallow rule for the sixth move at depth two is gone; the
reduced search keeps one ply rather than two; and a quiet check or a quiet
move into the enemy king's zone takes the table's reduction less one ply,
with the quiet floor of one ply still applying, instead of no reduction.
Castling, promotions, protected moves (the hash move, a previous principal
variation's move, a killer, and an attacking pawn push above Aggression 0) and
every move of a node in check are still searched at full depth, and
favourable captures are still never reduced.

## Fixtures that moved

Three regression scores moved with their moves unchanged: `win-hanging-queen`
486 to 485, `punish-central-queen` 1139 to 1169 and `contain-lone-rook` −521
to −518. Two standard-acceptance records were knife edges: in
`standard-avoid-equal-queen-trade` the objective profile now takes the queen
trade d2h6 into a bare-king draw, a move tied at 0 with the pinned d2b2 and
the move the suite's own objective column names, so bm0 accepts both; in
`standard-knight-d5-investment` the default profile plays g5f3 rather than
a2a3 at 20,000 nodes, with a root loss of 0 cp by the objective search and
still declining the c3d5 investment the fixture gates, so bm75 accepts both.
The personality, sacrifice-gate, null-move and UCI suites passed unchanged.
The two late-move-reduction unit tests were rewritten to the new rules.
