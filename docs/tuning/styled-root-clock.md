# Styled-root time cost at short time controls

This report measures the engine series ending at `833d3ef6f6572d3cb41e698d266b169dca5b6281`. The
default profile costs about 45 Elo against Aggression 0 at 50 ms per move and about 15 Elo at `10+0.1`.
Almost all of that difference is time the styled root spends probing alternatives, not the moves it
chooses. One change landed: the search forecasts its next iteration from the objective root search
alone. A second change, shallower styled-root budgets, was measured and not adopted.

## Diagnosis

Every row is a candidate against the unmodified engine at the default Aggression 75. Each candidate is
the same binary with one behaviour switched on by an environment variable, so the only difference is
the behaviour named. Games are paired and colour-reversed from
`docs/tuning/data/selective-search-confirmation.epd`, 4096 at 50 ms per move unless noted. The forcing
and check columns are the candidate's per-100-move rates divided by the baseline's in the same games,
counted as `tools/analyze_match.py` counts them. A control of the same binary against itself measured
+2.0 [-4.0, +8.1].

| Candidate | Elo | Forcing | Checks |
| --- | ---: | ---: | ---: |
| Aggression 0 | +45.3 [+39.1, +51.6] | 0.890 | 0.669 |
| Styled root off (conventional root only) | +38.5 [+32.3, +44.8] | 0.922 | 0.756 |
| Styled probing kept, objective move always played | +6.7 [+0.3, +13.1] | 0.908 | 0.744 |

Playing the objective move after all of the probing recovers only about a sixth of the strength that
removing the styled root recovers, so most of the cost is time. At `go movetime 50` over 150 book positions the mean
depth reached, over three alternating rounds, was 7.04 by default, 7.28 with the styled root off and
7.05 with the probing kept; at `go movetime 500` all three reached about 12.1. At `10+0.1`, Aggression
0 measured +14.9 [+5.4, +24.5] over 1280 games (forcing 0.900, checks 0.712) and the styled root off
measured +1.7 [-8.1, +11.5] over 1024.

The styled root spends node budgets that are capped absolutely, so its share of a search is large in
the shallow iterations a 50 ms search is made of and negligible in a deep one. The search also decides
whether to start another iteration by forecasting it as twice the last one plus 5 ms. The last
iteration's duration included the styled pass, so a small absolute cost was doubled and pushed the
forecast past the deadline: the mean depth was 0.24 plies lower, as if about a quarter of the searches
stopped an iteration early. The same rule leaves a fixed `movetime` search using 34 ms of a 50 ms
budget on average, with or without the styled root.

## Change

`search_root_styled` records when its objective root search finished, and `run_worker` measures the
iteration up to that instant. The styled pass still runs and is still charged to the clock; it is no
longer extrapolated. Searches without a styled root, helpers, and searches with no deadline are
unchanged. A fixed-depth search to depth 13 over 40 book positions visits exactly the parent's nodes at
Aggression 0, 75 and 100.

Measured in the switchable build, the change alone was +17.3 [+11.5, +23.2] at 50 ms with forcing 0.996
and checks 0.975.

## Result

The candidate is the committed engine and the baseline the unmodified `833d3ef` build, both Aggression
75. The style intervals are bootstrapped over colour-reversed opening pairs.

| Control | Games | Elo | Forcing | Checks |
| --- | ---: | ---: | ---: | ---: |
| 50 ms per move | 4096 | +15.6 [+9.6, +21.7] | 1.026 [1.007, 1.046] | 1.047 [0.998, 1.098] |
| `1.0+0.01` | 4096 | +3.6 [-2.3, +9.6] | 0.998 [0.977, 1.019] | 0.995 [0.946, 1.046] |

At `10+0.1` the styled pass is a millisecond or two against iterations of hundreds, so the forecast
moves by a few milliseconds and no match was played.

## Not adopted: shallower styled-root budgets

The second change lowered the depth scales of the styled-root node budget from 48 and 80 to 24 and 40
nodes per doubling and its floor from 256 to 128, keeping the ceilings at 2,048 nodes and 4,096 when a
sacrifice hint reserves more. Together with the forecast change, against the unmodified parent:

| Control | Games | Elo | Forcing | Checks |
| --- | ---: | ---: | ---: | ---: |
| 50 ms per move | 4096 | +25.6 [+19.6, +31.6] | 1.001 [0.983, 1.021] | 0.986 [0.941, 1.035] |
| `1.0+0.01` | 4096 | +12.4 [+6.2, +18.6] | 0.991 [0.970, 1.013] | 0.974 [0.925, 1.025] |
| 50,000 nodes per move | 4096 | +4.0 [-2.0, +10.0] | 0.984 [0.967, 1.002] | 0.954 [0.911, 0.998] |
| `10+0.1` | 1280 | +3.0 [-6.0, +12.0] | 0.972 [0.940, 1.006] | 0.921 [0.844, 1.008] |

The forecast change is inert at fixed nodes, so the 50,000-node row isolates the budgets. The gain is
confined to the shortest controls and the shortfall in forcing moves and checks grows as the search
lengthens, which is what fewer shallow probes warming the table for the deeper verification would
produce: at 400,000 nodes the `standard-opposite-castle-storm-deeper` position completes 4
verifications rather than 10. Halving the ceilings as well measured +26.6 [+20.4, +32.8] at 50 ms, the
same within noise, and cost more style.

The budgets also move fixed-node records that sit on the depth the search completes. At 20,000 nodes
`starting-style` scores 21 rather than 24 centipawns, and `standard-knight-e4-investment` plays `d2e4`
at 8 centipawns of root loss where `d2b1` is pinned. At 400,000 nodes `standard-opposite-castle-storm-deeper`
plays the objective `b2b4` where `a2a4` is pinned; the unmodified parent plays `b2b4` from 500,000 nodes.
Both acceptance records pass on both builds at 40,000 and 250,000 nodes. The change is the three
constants named above plus those re-pins, so anyone who wants the short-control gain and accepts that
style cost can apply it.

## Limitations

- The adopted change is a short-control change. Switching the styled root off measured +1.7 [-8.1,
  +11.5] at `10+0.1`, so there is little left for it to recover; what it does is let the 50 ms series
  measure the engine rather than the personality's overhead.
- Every figure is a single self-play run; 4096 games resolve about 6 Elo.
- Cost-of-style figures quoted at 50 ms include the overhead this report measures.

## Reproduction

```sh
cargo build --release --locked --bin jakgro --bin selfplay
python3 tools/run_sprt.py \
  --engine target/release/jakgro \
  --baseline-engine artifacts/baseline-jakgro \
  --candidate-aggression 75 --baseline-aggression 75 \
  --games 4096 --movetime-ms 50 --concurrency 88 \
  --elo0 0 --elo1 5 \
  --openings docs/tuning/data/selective-search-confirmation.epd \
  --pgn artifacts/styled-root-clock-50ms.pgn
python3 tools/analyze_match.py --pgn artifacts/styled-root-clock-50ms.pgn
```

Repeat with `--time-control 1.0+0.01` for the clocked row.
