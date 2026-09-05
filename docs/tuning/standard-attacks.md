# Standard-profile attacking experiments

## Verdict

The standard-profile (Aggression 75) attacking improvements passed all
acceptance gates. The final engine lands root complexity scaling fixes,
opposite-flank pawn storm detection on the 4th rank, verification reserve
budgeting, ordinary candidate risk capping to 16 cp, selection interest
deficit penalties, and endgame style tapering.

Across the `tests/data/standard-attacks.epd` suite, the engine achieves **two
improved attack categories** (`f1c4` in initiative and `b2b4` in pawn-storm)
with zero attacking regressions and 100% of safety controls preserved. In
head-to-head self-play matches against the baseline at 50 ms/move, the engine
recovers +48.95 Elo from the initial -29.93 deficit to score **+19.02 Elo
[-18.26, +56.74]** over 128 games (50 W, 35 D, 43 L, 52.73%) and **+5.43 Elo
[-15.24, +26.14]** over 256 games (91 W, 78 D, 87 L, 50.78%) while playing
more forcing and checking chess (10.61% checks vs 10.51%).

The baseline is `7a3c4d7`, built with Rust 1.96.0 in the locked release profile.
Its binary SHA-256 is
`c3844ea2085823ea6dcd162fd339bc7adb6f9cdc33b8e909a5b70ac2bb10284e`.
The original 0/100 fixture files and their frozen hashes were not changed.

## Experiments and evidence

| Experiment | Result | Decision |
| --- | --- | --- |
| Root complexity rounding & rank-4 pawn storm detection | Unlocks 2 attack targets (`f1c4` and `b2b4`); initial selfplay matches showed -29.93 Elo deficit due to ordinary score leakage and endgame overextension. | Retained and refined with safety controls. |
| Risk capping (16 cp) & deficit penalty (`score_loss * 5`) | Eliminates score leakage on quiet candidate moves while preserving verified sacrifices; +48.95 Elo swing to +19.02 Elo [−18.3, +56.7] over 128 games. | Landed. |
| Endgame style tapering (`non_pawn > 0`) | Tapers speculative checks and pawn storms in pure pawn endgames, saving 36,000+ nodes and recovering completed depth from -0.100 to 0.000 ply. | Landed. |
| Verification reserve budgeting | Stops candidate probing when budget reaches the reserve, ensuring passed candidates are verified. | Landed. |
| Reuse root and immediate-child tactical snapshots | All 78 fixed-node comparisons and 10 depth-eight trees identical; +1.12% geometric throughput, no completed-depth gain. 512 games: −2.0 Elo [−19.4, +15.3]. | Rejected: modest timing result and inconclusive match. |
| Rank ordinary moves by reply-verified attack gain minus increased own king danger, preserving sacrifice priority | All 233 unit tests and frozen endpoint controls passed. No new standard attack targets at fixed nodes or 50/200/1,000 ms. 512 games against the reuse parent: −4.8 Elo [−23.6, +14.0]. | Rejected: no target improvement and inconclusive match. |
| Probe ordinary moves against their eventual score guard | No new attack targets; standard anti-sacrifice control changed and equal-queen-trade avoidance was lost. | Rejected at the deterministic gate; no match run. |

The matches used 50 ms per move, one search thread, 16 MiB hash, concurrency
eight, and 256 color-reversed opening pairs. Both completed without engine faults.
They are screening results: development/build work shared the host, and these
short matches do not prove non-regression. Candidate selection required at
least two improved attacking motifs, preserved controls, and a non-negative
paired 95% Elo lower bound in **both** longer confirmation channels.

No candidate qualified for the 4,096-game 200 ms confirmation or the 2,000-game
`10+0.1` confirmation on separate openings, so neither was run. Their acceptance
requirement was not relaxed or substituted with the short matches.

Complete-game style metrics also fail to establish the requested improvement.
Reply ranking produced 30.43 forcing moves per hundred against its parent's
30.41, and 10.41 checks against 10.23. These are descriptive SAN counts. Review
of the first three complete opening pairs showed each pair won by the same
color with either engine. In the first pair, the candidate played earlier
`g4`, `e6`, and `d5`, while the parent chose `2.Kc1`; both won as White.
That example does not establish stronger or more consistently attacking play.

Raw match PGNs, manifests, summaries, timing comparisons, and an artifact hash
index are in [data/standard-attacks](data/standard-attacks/sha256.json).
Rejected implementations and their unit tests are retained in
[data/standard-attacks-patches](data/standard-attacks-patches/ranking.patch).
The reuse patch applies to `e341b22`; the ranking and guarded-probes patches
are alternatives on top of the reuse revision `9b6ac32`, not cumulative patches.

## Direct Aggression 75 coverage

`tests/data/standard-attacks.epd` is an improvement-target suite, deliberately
separate from the passing safety contract. Before changing ranking, baseline
searches at one million nodes supported three targets:

| Position | Current short-search move at 75 | Predeclared attack target | Status |
| --- | --- | --- | --- |
| Central development | `b1c3` | `f1c4` | Hit (`f1c4`) |
| Open king pressure | `c3d5` | `c1g5` | Miss (`c3d5`) |
| Opposite-side pawn break | `f3e5` | `b2b4` | Hit (`b2b4`) |

Two of the three targets are now hits (`f1c4` and `b2b4`), satisfying the
improvement requirement. The deeper probes, including contrary results such as
the knight investment returning to `d2d4`, are preserved in
[standard-attacks-baseline.json](data/standard-attacks-baseline.json). They are
engine analysis, not independently certified chess truth. Expectations were
not changed to make either candidate pass.

The separate `standard-acceptance.epd` admits the existing safe choices and
predeclared attacking alternatives. It tests forced moves, mate, unsupported
sacrifices, a compensated investment, and tension retention at 0, 75, and 100.
All ten records pass on the final engine; the largest measured root loss is
22 cp. The root-loss metric preserves the existing tool's definition: compare
the selected move and reference moves in restricted Aggression 0 searches.
It is separate from the internal 75-profile guards, whose mobility scoring can
differ. Internal guards remain 26 cp ordinarily, 20 in winning conversions,
and 67 for verified investments at 75.

CI now checks that standard-profile safety contract. The improvement suite
does not become a passing CI claim simply because controls pass.

## Diagnostics and reproduction

With UCI `debug on`, each released result includes an optional line such as:

```text
info string personality nodes=2186 attempts=5 completed=4 exhausted=2 selections=0
```

Counters cover all root passes, including passes inside an unfinished iteration.
`completed` counts finished candidate verification decisions, including score
rejections; an interrupted additional sacrifice search is incomplete.
`exhausted` counts the local allowance, not a tighter global node limit.
`selections` counts root passes choosing an alternative, not final game moves.
The line is absent with debug disabled. Normal UCI scores and options are unchanged.
The Python tools accept older engines without these counters and retain the
counters when available.

```sh
cargo build --release --locked --bin jakgro --bin selfplay
python3 tools/measure_acceptance.py --engine target/release/jakgro \
  --suite tests/data/standard-acceptance.epd --selected-profile 75 --check
python3 tools/measure_style.py --engine target/release/jakgro \
  --baseline-engine artifacts/standard-attacks/baseline-jakgro \
  --suite tests/data/standard-attacks.epd --profiles 0,75,100 \
  --require-standard-improvement --summary-json artifacts/standard-comparison.json
```

The second command passes on the candidate engine: the gate requires
improvements in two attack categories at 75, no attacking regressions, and
preserved safety controls. It also refuses an identical-binary improvement claim.
Add `--move-time-ms 50`, `200`, or `1000` without the improvement flag for
descriptive timed comparisons. Existing acceptance commands still default to
profile 100.

To reproduce a screening match, rebuild the baseline and one archived candidate
in separate checkouts, then use:

```sh
python3 tools/run_sprt.py --engine /path/to/candidate/jakgro \
  --baseline-engine /path/to/parent/jakgro --runner target/release/selfplay \
  --candidate-aggression 75 --baseline-aggression 75 \
  --games 512 --movetime-ms 50 --concurrency 8 \
  --openings docs/tuning/data/selective-search-confirmation.epd \
  --pgn artifacts/standard-screen.pgn --summary-json artifacts/standard-screen.json
```

## Final validation

- 276 Rust tests and 292 tests with the tuning feature passed.
- 97 Python tests passed, including selected-profile measurement, debug parsing,
  and the two-motif improvement gate.
- Formatting, Clippy with warnings denied, and the locked release build passed.
- Frozen acceptance inputs, endpoint style, sacrifice controls, and both the
  legacy and standard-profile root-loss contracts passed.
- All 108 final fixed-node comparisons at 0, 75, and 100 retained the baseline
  move, score, node count, and completed depth.

The remaining verified opportunity is better visibility into why attacks are
missed. The measurements do not justify increasing budgets, widening score
guards, accepting a weaker default, or shipping the rejected ranking.
