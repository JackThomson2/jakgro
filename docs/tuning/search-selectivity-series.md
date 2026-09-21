# Search selectivity series

## Verdict

Eight search patches landed out of eighteen measured (one of them, the improving signal, as a neutral simplification). Measured head against
the shipped base (`878e369`, the 192-group network) over 4096 colour-reversed
games at a fixed 50 ms per move, the series is worth **+70.4 Elo [63.8,
77.1]** at Aggression 75, accepting H1 with no faults. A clocked channel at
`1.0+0.01` over 2048 games measures **+69.8 Elo [60.4, 79.3]**, accept H1.

The engine did not become duller for it. Over 4096 Aggression 75 versus 0
games at 50 ms the head plays 30.20 forcing moves per hundred against 27.76
for its objective profile, a ratio of 1.088 against the base's 1.062 on the
same match, and 12.13 checks per hundred against the base's 11.16. The measured
cost of the default profile against objective play is -40.0 Elo [-46.5,
-33.5] on the head against -35.8 Elo [-42.5, -29.2] on the base.

## Why this series

The tree was the bottleneck, not the evaluation. Before the series the engine
reached depth 10 from the starting position at 542k nodes, an effective
branching factor of 2.1–2.5 per ply; quiescence was 76% of every fixed-depth
tree and reached depth 8 on the probe suite in 1.47M nodes. After the series
the same suite takes 0.72M nodes at depth 8. Every patch was measured by a
fixed-time paired match against the same frozen base binary rather than by the
frozen-suite depth screen, and the personality fixtures were re-pinned where
the deeper trees moved them rather than vetoing the change.

## Per patch

Each row is the head with that patch against the frozen base at 50 ms per
move, 4096 games; the gain over the previous accepted patch is the difference
between consecutive rows (about ±9 Elo on that difference).

|Patch|Elo vs base|Nodes at depth 8|Forcing ratio|
|---|---|---|---|
|Base|0|1.467M|1.080|
|Quiescence: delta pruning, SEE < 0 pruned, quiet checks in the first budget plies only|**+18.3 [11.6, 24.9]**|1.214M|1.082|
|Store fail-low results after pruning|+15.6 [9.2, 22.1], rejected|1.170M|1.032|
|Static pruning to depth 8, quiet futility to depth 6 at 100 + 100·depth, no material condition, move-count formula (3 + d²)/(2 − improving), improving from every static evaluation|+6.2 [-0.5, 12.9], rejected as a bundle|0.840M|1.068|
|The same without the move-count formula and improving change|**+26.7 [20.2, 33.2]**|1.127M|1.046|
|Improving derived from every static evaluation|**+24.8 [18.2, 31.4]**, kept as neutral|1.059M|1.093|
|LMR: one more ply when not improving; losing captures reduced one ply less than quiets|**+35.2 [28.4, 41.9]**|1.009M|1.080|
|Aspiration 16 cp, one-sided widening, from depth 4|+25.2 [18.6, 31.9], rejected|1.002M|1.035|
|Aspiration 30 cp, otherwise as above|+25.8 [19.4, 32.3], rejected|—|1.080|
|History pruning (< −HISTORY_MAX/8·depth to depth 4) and swap-list pruning of quiets (−25·d²) and captures (−150·d) to depth 6|+34.5 [27.8, 41.2], rejected as neutral|0.874M|1.056|
|Null move: table and pruning inside probes, min depth 3, R = 3 + d/3 + excess, verification from depth 12|−16.4 [−23.0, −9.7], rejected|0.686M|1.058|
|Table and pruning inside probes alone|+25.6 [18.9, 32.3], rejected|0.900M|1.061|
|LMR: one more ply at cut nodes|+22.7 [16.0, 29.4], rejected|0.990M|1.087|
|Null probe under the null board's key; table, ordering and pruning inside probe and verification|**+40.6 [34.0, 47.3]**|0.842M|1.063|
|Null move min depth 3, R = 3 + d/3 + min((eval − beta)/200, 3), verification from depth 12|**+45.8 [39.2, 52.5]**|0.661M|1.076|
|Internal iterative reduction at depth ≥ 4 without a hash move|**+57.8 [51.0, 64.7]**|0.531M|1.084|
|Singular extension from depth 6, margin 2·depth, multi-cut at non-PV nodes|+57.8 [51.2, 64.4], neutral|0.591M|1.086|
|Singular extension from depth 4|**+70.4 [63.8, 77.1]**|0.717M|1.082|
|Correction history keyed by pawn structure on the main-search static evaluation|+63.9 [57.0, 70.8], rejected|0.732M|1.093|

The forcing ratio column is from 1024-game matches and carries about ±0.03
of noise; the 4096-game figures in the verdict are the ones to cite.

## What the null-move rewrite found

The first null-move rewrite lost 52 Elo against its parent, and half of it
survived bisection into "let the probe use the table". The probe searched the
null board without pushing that board's repetition key, so it probed and
stored under the parent's key with the side to move inverted. The old
`NullProbe` mode never touched the table, which is why the bug had no effect
until the mode was allowed to. Pushing the key turned the same change from
−10 into +5 Elo, and the policy change that had measured −52 on top of the
bug measured +5 on top of the fix.

## Rejected directions

- Narrow aspiration windows lost about ten Elo at both 16 and 30 cp with
  one-sided widening from depth 4; the 50 cp symmetric window stays.
- Storing fail-low results at nodes where a move was pruned did not gain and
  is not kept; the search still refuses those stores.
- One more ply of reduction at cut nodes lost twelve Elo; the log table with
  the improving and history adjustments is the whole reduction.
- History and swap-list pruning of late moves shrank the tree 13% and gained
  nothing at 50 ms; the swap-list cost ate the saving.
- Correction history on the main-search static evaluation lost six Elo. The
  fourth series' attempt on the quiescence stand-pat lost eight.

## Fixtures

The deeper trees at the fixture node budgets moved three regression scores
and four profile fixtures. `unsupported-bishop-f6` retreats the bishop at
both profiles (the anti-sacrifice contract, agreement, holds and is
re-pinned). `avoid-queen-trade-for-knight-d5` and `standard-queen-d5-investment`
resolve as clearly won and no longer separate the profiles; they are replaced
by positions mined from the self-play corpus at 20k nodes: an equal
middlegame where Aggression 0 trades queens and 100 advances a pawn (0 cp
objective loss), and a knight-for-pawn investment the default profile plays
within 80 cp of the objective move. `queenside-pawn-thrust` had swapped roles
and is replaced by a queenside thrust the attacking profile prefers to a
knight move. The null-move contract test no longer requires move agreement
with an unpruned search, since fail-highs below depth twelve are trusted; it
keeps a one-pawn score bound, the telemetry invariants, legal PVs and the
node saving.

## Reproduction

```sh
cargo build --release --locked --bin jakgro --bin selfplay
python3 tools/run_sprt.py --runner target/release/selfplay \
  --engine target/release/jakgro --baseline-engine <base-878e369> \
  --candidate-aggression 75 --baseline-aggression 75 \
  --games 4096 --movetime-ms 50 --concurrency 88 --elo0 0 --elo1 10 \
  --openings docs/tuning/data/selective-search-confirmation.epd \
  --pgn artifacts/search/strength.pgn
python3 tools/run_sprt.py --runner target/release/selfplay \
  --engine target/release/jakgro --candidate-aggression 75 --baseline-aggression 0 \
  --games 4096 --movetime-ms 50 --concurrency 88 \
  --openings docs/tuning/data/selective-search-confirmation.epd \
  --pgn artifacts/search/style.pgn
python3 tools/analyze_match.py --pgn artifacts/search/style.pgn --json artifacts/search/style.summary.json
```

These are short-control relative results against one base binary on one
host, not an absolute rating.
