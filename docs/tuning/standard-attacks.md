# Standard-profile attacking experiments

## Verdict

No playing change passed the agreed acceptance gates. The final engine keeps
the baseline's evaluation, move selection, pruning, score guards, and personality
budget. This series adds direct Aggression 75 coverage and verification telemetry;
it makes **no strength or increased-aggression claim**.

The baseline is `7a3c4d7`, built with Rust 1.96.0 in the locked release profile.
Its binary SHA-256 is
`c3844ea2085823ea6dcd162fd339bc7adb6f9cdc33b8e909a5b70ac2bb10284e`.
The original 0/100 fixture files and their frozen hashes were not changed.

## Experiments and evidence

| Experiment | Result | Decision |
| --- | --- | --- |
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

| Position | Current short-search move at 75 | Predeclared attack target |
| --- | --- | --- |
| Central development | `b1c3` | `f1c4` |
| Open king pressure | `c3d5` | `c1g5` |
| Opposite-side pawn break | `f3e5` | `b2b4` |

All three remain misses. The deeper probes, including contrary results such as
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

The second command intentionally fails on the current engine: the new gate
requires improvements in two attack categories at 75, no attacking regressions,
and preserved safety controls. It also refuses an identical-binary improvement
claim. Add `--move-time-ms 50`, `200`, or `1000` without the improvement flag
for descriptive timed comparisons. Existing acceptance commands still default
to profile 100.

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
