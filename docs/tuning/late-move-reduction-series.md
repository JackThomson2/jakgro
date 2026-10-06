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

## Follow-ups measured on the series head

Each row is a candidate against the head that carries the patch above
(`a48568c`), Aggression 75 on both sides, 4096 paired games per match; style
ratios as above.

### Re-search depth adjusted by the reduced score (audit S3): rejected

After a reduced search raised alpha, the full-depth repeat went one ply
deeper when the reduced score beat the best score so far by a margin and one
ply shallower when it barely did, so that a one-ply reduction whose score
barely rose was not repeated at all; the principal-variation re-search used
the adjusted depth too. This is Stockfish's `doDeeperSearch` /
`doShallowerSearch` pair. Three margin settings were screened and matched.

| Deeper / shallower margin (cp) | Nodes at depth 13, Aggression 75 / 0 | 50 ms | 10+0.1 |
|---|---|---|---|
| 50 / 8 | 21,158,384 / 18,367,472 (−7.2% / −7.3%) | −2.5 [−8.8, +3.7] | +1.2 [−4.0, +6.3] |
| 50 / off | 24,216,476 / 19,525,568 (+6.2% / −1.5%) | −4.3 [−10.5, +1.8] | — |
| 25 / 8 | 22,294,859 / 17,966,720 (−2.2% / −9.3%) | −5.1 [−11.1, +0.9] | — |

The node savings did not convert: every form lost at 50 ms and the best of
them was null at 10+0.1 (forcing 0.992 [0.975, 1.010], checks 0.982 [0.937,
1.029]). With reductions still much milder than Stockfish's, a reduced search
that raises alpha is rarely wrong enough to deserve a deeper repeat and
rarely so marginal that skipping the repeat is safe; the device belongs to a
search that reduces harder. Not kept.

### The forcing-order bonus bounded inside the history range (audit O3): kept

Above Aggression 0 the quiet ordering gave every quiet check
`800,000 + 3,000 × aggression` and every attacking pawn push
`1,500 × aggression`, on a history scale of ±16,384: at the default profile a
check ranked above the killers and a push above every quiet of ordinary
history, however the check hung its piece or however strong the history of
the quiet it displaced. The audit's ablation had measured the bonus switched
off at +5.9 [−0.5, +12.3] with the forcing and check rates intact, the only
personality knob whose removal left the style alone. Three forms were
screened on the series head.

| Form | Nodes at depth 13, Aggression 75 | 50 ms | Forcing / checks at 50 ms | 10+0.1 | Forcing / checks at 10+0.1 |
|---|---|---|---|---|---|
| bonus within the history range: safe check `110 × aggression`, push `55 × aggression`, killers first | 22,790,899 (−0.02%) | not matched | — | — | — |
| bonus off | 23,896,729 (+4.8%) | +6.6 [+0.4, +12.8] | 0.986 [0.967, 1.005] / 0.966 [0.920, 1.012] | — | — |
| **bounded as above, and no bonus for a check whose destination is en prise** | 23,635,857 (+3.7%) | **+6.9 [+0.8, +13.0]** | 1.001 [0.983, 1.020] / 1.007 [0.960, 1.054] | +3.6 [−1.4, +8.7] | 1.001 [0.983, 1.019] / 1.005 [0.960, 1.052] |

The bounded bonus alone leaves the tree all but unchanged, because history
values at depth 13 rarely exceed half the range and a check still leads;
the gain needs the swap-list gate. A check that hangs the checking piece now
earns nothing and is ordered by its history like any other quiet, while a
safe check leads the quiets of ordinary history and yields to a killer or to
a quiet with a strong record. Switching the bonus off entirely gains as much
Elo but reads three to four percent fewer checks at 50 ms; the gated form
keeps both rates at 1.00. At 10+0.1 the gated form reads +3.6 [−1.4, +8.7]
with both rates still at 1.00; the two matches together put it at about
+5.0 ± 3.9 over 8192 games, and it is kept on that pooled evidence. The bonus
is read at two places, the staged picker's quiet generation and the eager
ordering of root and verification searches, and the swap list runs only for
the checking quiets, a few percent of the quiets generated. The deeper trees
moved two regression scores with their moves unchanged, `win-hanging-queen`
485 to 499 and `contain-lone-rook` −518 to −529, and three
standard-acceptance records at the default profile that probing both
binaries across budgets showed to be knife edges: in
`standard-open-king-gambit` the default plays a2a3 from 150,000 to 400,000
nodes, three centipawns below c3d5, which the head itself plays at 300,000
and both return to at 800,000, so bm75 adds a2a3 to the quiet alternatives
it already accepted; and in the opposite-castle-storm position the default
alternates between the storm pushes a2a4 and b2b4 from 10,000 to 800,000
nodes on both binaries, while profile 0 plays b2b4 throughout, so both
records accept both pushes. The personality, sacrifice-gate, null-move and
UCI suites passed as they were.
