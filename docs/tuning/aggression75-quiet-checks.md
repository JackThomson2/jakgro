# Aggression 75: spend less on optional quiescence checks

## Verdict

This follow-up measures **a further +44.9 Elo at 50 ms/move and +39.6 Elo at
`1.0+0.01`**, against the already-improved engine from
[the fitted-mobility series](aggression75-fitted-mobility.md). Both engines use
Aggression 75. These are incremental short-control comparisons, not an absolute
rating or a measured cumulative gain against the older pre-series engine.

| Match | Games | W / D / L | Score | Elo, paired normal 95% interval | Final LLR |
| --- | ---: | ---: | ---: | --- | ---: |
| Screening, 50 ms | 1,024 | 397 / 339 / 288 | 55.322% | +37.1 [+24.0, +50.3] | 7.261 |
| Confirmation, 50 ms | 3,072 | 1,255 / 957 / 860 | 56.429% | **+44.9 [+37.6, +52.3]** | 29.086 |
| Confirmation, `1.0+0.01` | 2,000 | 856 / 515 / 629 | 55.675% | **+39.6 [+30.0, +49.2]** | 14.772 |

All three runs completed without engine faults and accept H1 for the configured
[0, 10] Elo test with alpha=beta=0.05. `run_sprt.py` evaluated the LLR at the
preselected game caps; these were not early-stopped runs. The conservative
Hoeffding intervals in `analyze_match.py` also exclude zero for both
confirmations: **[+20.6, +69.7]** and **[+9.6, +70.2]**. They are retained alongside
the narrower pair-variance normal estimates rather than substituted for them.

## The engine change

Delay the second optional quiet check in quiescence from aggression 50 to 80:

| Aggression | Quiet-check allowance per quiescence line |
| --- | ---: |
| 0–79, including the default 75 | 1 |
| 80–99 | 2 |
| 100 | 3 |

Previously 50–99 all received two. Thus this changes profiles 50–79, not only
75, although the strength measurements cover 75 only. The schedule remains
bounded and monotone, including clamping above 100.

Quiescence searches tactical continuations beyond the main search horizon.
Reducing its optional quiet checks is **not** skipping normal-search checks or
legal check evasions. Captures and promotions do not spend the quiet-check
allowance, and a checked quiescence position still searches legal evasions even
when both its depth and quiet-check budgets are exhausted. A new regression
explicitly exercises quiet evasions with both budgets zero.

The default still has three main-search check extensions. Root attack interest,
move-ordering bonuses, pawn-storm protection, score margins, draw/simplification
preferences and full sacrifice verification are unchanged. The fitted evaluation
and default setting of 75 are also unchanged. No production search routine was
rewritten; the production change is the allowance returned by
`EvaluationConfig::quiescence_check_budget`.

On the ten-position efficiency suite, at depth eight and seven alternating
samples with 1,000 ms probes, the candidate searches **10.96% fewer nodes**, gains
**0.2 completed ply** on average, and reports +1.06% fixed-node throughput.
The trees differ; those numbers are not a pure speedup and do not substitute for
the match results. The useful trade is spending less search on optional horizon
checks while preserving the attacking choice policy.

## Style and safety

| Per 100 moves | Candidate | Baseline | Retention |
| --- | ---: | ---: | ---: |
| Forcing moves, 50 ms confirmation | 30.860 | 30.823 | **100.1%** |
| Checks, 50 ms confirmation | 10.344 | 10.750 | 96.2% |
| Forcing moves, clocked confirmation | 31.534 | 30.968 | **101.8%** |
| Checks, clocked confirmation | 11.828 | 11.813 | 100.1% |

These are SAN-derived descriptive rates, not independent move-quality judgments.
The engine is not simply becoming quieter: forcing rates are retained, and
clocked checking rates are essentially unchanged. At fixed nodes, the default
retains `c1g5` in the open-king attack, `b2b4` in the opposite-flank pawn storm,
`e5c6` in the compensated investment, and `d2e2` to avoid the equal queen trade.
All forced tactical/defensive choices remain intact.

### One explicitly revised non-forced control

**The candidate does not pass the original standard-profile acceptance suite
unchanged.** In `standard-unsupported-greek-gift`, it chooses `f1e1` rather than
`b1d2` at 100,000 nodes. Both are ordinary development moves; neither is the
unsupported `d3h7` sacrifice. The original fixture allowed only `b1d2` and required
zero loss against its restricted objective reference. The new choice measures
1 cp lower there.

Only that standard-profile record is revised: `bm75` admits both `b1d2,f1e1`,
and its reference-loss cap changes from **0 to 1 cp**. The node budget, objective
reference, and 0/100 expected moves are unchanged. No forced-defense allowance,
legacy endpoint fixture, legacy sacrifice fixture, or null-move contract is
relaxed. This is a deliberate, reviewable tolerance change, not a claim that the
old gate passed.

Both full searches return to `b1d2`, +63 cp, at two million nodes. Separate
restricted **Aggression 0** searches provide additional context:

| Nodes per restricted move | `b1d2` | `f1e1` | Unsupported `d3h7` |
| ---: | ---: | ---: | ---: |
| 100,000 | +50 | +49 | -274 |
| 500,000 | +63 | +59 | -274 |
| 2,000,000 | +56 | +57 | -261 |

These are engine probes, not certified chess truth. They support admitting the
rook development without admitting the sacrifice: the latter remains over
three pawns below either quiet option. Both the original **failed** acceptance
report and the revised passing report, with their exact before/after suites,
are archived. Maximum measured root loss over the revised standard suite remains
22 cp on the intended compensated investment.

## Validation

- **280 Rust tests passed**, including the bounded/monotone schedule and exhausted
  quiescence evasion regression.
- **296 tests passed with the tuning feature**.
- Formatting and Clippy with warnings denied passed.
- **12 Python acceptance-contract/measurement tests passed**.
- The frozen acceptance-input validator, legacy acceptance, endpoint style and
  sacrifice gates pass. All ten records in the explicitly revised standard
  acceptance suite pass.
- All **60 fixed-node endpoint comparisons** at 0 and 100 preserve completed
  iteration scores, nodes, PVs, best moves and personality counters.
- All **seven ordinary 20,000-node regression moves and scores are unchanged**;
  no ordinary regression record was re-pinned. Probes at 100,000 and 400,000 nodes
  are retained, including positions where the candidate completes another ply.
- The rebuilt committed engine has exactly the same SHA-256 as the immutable
  binary used for all matches.

The default and tuning test runs used `NEXTEST_TEST_THREADS=4`. Full raw Rust
validation output is archived. The full Python suite was not rerun; its tools
were not changed.

## Provenance and limitations

| Input | Value |
| --- | --- |
| Series base | `a0d24fbd77878b208ce9a86ac10e8312a88a8500` |
| Engine patch | `c957c4d94f58c6f73955ab9f0501b2fed59ce8ef` |
| Base binary SHA-256 | `aab08df1d6e479010d68d8feba5a604db58393acf7d202868ad41e8e46fc497f` |
| Candidate binary SHA-256 | `761fbb44921e9bb7eff967b4aae603f47c9c7871297229848e9cc9b062c7a009` |
| Dependency revision | `7e93cdea094a50c1574081ceb6e7b269ad0234ee` |
| Toolchain | Rust/Cargo 1.88.0, locked release build, fat LTO |
| Host | Linux x86_64, 96 online logical CPUs |
| Search / match settings | One thread per engine, 16 MiB hash, concurrency 24 per match |

The screen used the first 512 pairs of the existing 2,048-position corpus.
Confirmation used the remaining 1,536 positions stored in
`data/aggression75-fitted-mobility/a75-heldout.epd`, SHA-256
`5e11312942869d196476eac6e4f493596f1b2e81f11271cfc4152a163e99a9ec`.
The clocked channel uses the first 1,000 of those pairs. These positions were
excluded from **this round's screen**, but were used in the preceding series.
They are not fresh opening families, independent external opponents, or a newly
unseen book.

The two confirmation matches overlapped, and builds/checks shared the host.
This was not a dedicated timing machine. The candidate's revision label in the
original manifests is `q1-screen`; `provenance.json` maps that immutable binary
to the subsequently committed, byte-identically rebuilt source. Manifests are
kept unmodified.

Longer time controls, external books/opponents, multi-threaded strength, other
intermediate aggression settings, and current 75-versus-0 personality cost remain
unmeasured. Do not infer an absolute rating or add this gain to previous series'
point estimates as if a cumulative comparison had been run.

## Reproduction and archive

Build the base and candidate in separate checkouts. The confirmation command is:

```sh
cargo build --release --locked --bin jakgro --bin selfplay
python3 tools/run_sprt.py \
  --engine target/release/jakgro --baseline-engine /path/to/base/jakgro \
  --candidate-aggression 75 --baseline-aggression 75 \
  --games 3072 --movetime-ms 50 --concurrency 24 --hash 16 \
  --elo0 0 --elo1 10 \
  --openings docs/tuning/data/aggression75-fitted-mobility/a75-heldout.epd \
  --pgn /path/to/scratch/followup-q1-confirm.pgn
```

For the clocked channel replace `--games 3072 --movetime-ms 50` with
`--games 2000 --time-control 1.0+0.01`. For screening use the original
`selective-search-confirmation.epd` and 1,024 games.

The [artifact index](data/aggression75-quiet-checks/sha256.json) binds complete
compressed PGNs, manifests, arbiter results, logs, analysis reports, the original
and revised acceptance inputs/results, deeper probes, endpoint comparisons,
efficiency data and validation output. The archived endpoint-comparison script
expects `JAKKOO_TEMP` to contain rebuilt binaries named `followup-base` and
`followup-q1`, and is run from the repository root.

To audit results without replaying games, decompress each PGN into scratch and
copy its original manifest beside it. `tools/analyze_match.py` validates the
manifest and computes W/D/L and style rates. The recorded normal intervals and
LLRs can be recomputed with `tools.run_sprt.pair_points_from_pgn` and
`tools.run_sprt.evaluate`, using 0, 10, 0.05, 0.05. All 6,096 archived games and
33 indexed file hashes were audited this way before export.

The read-only investigation also identified branch-specific tactical snapshots
and unused conversion metadata as possible future optimizations. Neither was
implemented or assigned an Elo gain in this series.
