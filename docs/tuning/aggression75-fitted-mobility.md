# Aggression 75: fitted mobility and cheaper attack preparation

## Result

The two engine patches improve measured short-control strength against the
pre-series engine at **Aggression 75 on both sides**, without disabling its
attacking root policy. They do not establish an absolute Elo rating.

| Held-out comparison | Games | W / D / L | Score | Relative Elo, paired normal 95% interval | Final LLR, [0, 10] test |
| --- | ---: | ---: | ---: | --- | --- |
| 50 ms per move | 3,072 | 1,087 / 968 / 1,017 | 51.139% | **+7.9 [+0.09, +15.8]** | 1.828, continue |
| `1.0+0.01` | 2,000 | 790 / 514 / 696 | 52.350% | **+16.3 [+6.4, +26.3]** | 4.426, accept H1 |

Both matches completed with **zero engine faults**. The clocked result clears
the configured H1 boundary; the fixed-movetime result does not. Its confidence
interval only just excludes zero. Do not describe both as SPRT passes, add their
Elo estimates together, or extrapolate them to tournament controls.

The intervals above are the pair-variance normal estimates from
`tools/run_sprt.py`. The accompanying `analyze_match.py` reports also retain the
more conservative Hoeffding intervals: [-16.2, +32.1] and [-13.5, +46.4]. Those
intervals cross zero. The LLR was evaluated at each preselected game cap; these
runs did not stop early at a boundary.

## What changed

### Cheaper preparation, identical decisions at fixed nodes

Sacrifice seed ranking consumed only the style portion of a full tactical
snapshot. Preparing that snapshot also counted legal checks and settled
unrelated exchanges, neither of which the hint used. Seed ranking now requests
only style information, and requests no child snapshot when the target-specific
material offer is below the sacrifice threshold. Actual sacrifice verification
still performs full legal tactical settlement.

A direct reference test compares the old and new hint calculation across both
colors, checks, offers, en passant and pinned-piece positions. All **90**
comparisons over the personality, standard-attacks and sacrifice suites at
0/75/100 preserve every completed iteration's score, nodes and PV, the best move,
and personality counters. The ten-position depth-eight suite is also identical.
At Aggression 75 its seven-sample efficiency run measures **+4.15% throughput**
with no completed-depth gain. That is a speed measurement, not an isolated Elo
claim.

### Use the fitted mobility rather than the old overlay

The per-profile linear mobility adjustment was selected when mobility still had
uniform 3/2 middlegame/endgame weights. Later fits introduced per-piece mobility
curves and unsafe-square penalties, but deliberately excluded the old overlay
from their optimization. The default profile therefore added an independently
chosen linear adjustment to the fitted objective score.

All profiles now use the same fitted objective evaluation. The overlay, its
configuration field and its obsolete tests are removed; tests instead require
identical objective scoring across profiles while preserving the attacking
style channel. No fitted parameter, piece-square table or tuning-vector layout
is changed. Both objective evaluation paths and the fitter remain covered.

The default stays at **75**, with the same root interest, sacrifice priority,
score guards, verification reserve, draw/simplification preferences, three
check extensions and two quiescence quiet-check allowances. The verified
investment ceiling remains 67 cp. The ordinary 16 cp and winning-conversion
20 cp rules are unchanged, including their existing discontinuity.

## Attacking identity, not identical moves

| Complete-game measure, per 100 engine moves | Candidate | Baseline | Retention |
| --- | ---: | ---: | ---: |
| Forcing moves, 50 ms | 30.687 | 31.152 | **98.5%** |
| Checks, 50 ms | 10.262 | 11.036 | 93.0% |
| Forcing moves, clocked | 30.082 | 31.234 | **96.3%** |
| Checks, clocked | 10.506 | 11.980 | 87.7% |

The candidate is **less check-heavy**, especially in the clocked channel. These
SAN counts are descriptive proxies, not evidence that an individual check was
sound or unsound. Both forcing rates exceed the repository's 90% retention
floor. This is a strength/style trade, not a claim of more aggressive play.

All frozen endpoint and sacrifice controls pass, as do all ten standard-profile
acceptance records; maximum measured objective root loss remains **22 cp**.
The final engine also preserves all **60** fixed-node endpoint trees at 0/100.
At 75 the following deliberate changes are visible:

- central development changes from `f1c4` to `b1c3`;
- open king pressure changes from `c3d5` to the attack target `c1g5`;
- the `b2b4` opposite-flank pawn storm, `e5c6` compensated investment, and
  `d2e2` queen-exchange avoidance remain;
- forced moves and unsupported-sacrifice controls remain intact.

The old standard-attacks improvement-target gate is **not** claimed as a pass:
the central-development target is now missed. Its expectations were not changed
to hide that tradeoff. This series preserves attacking intent, not every old
style choice.

## Screening and attribution

All screens used the first 512 opening pairs at 50 ms, 1,024 games, concurrency
24, one search thread and 16 MiB hash. Their intervals are inconclusive.

| Candidate versus immediate parent | Elo, paired 95% interval | Decision |
| --- | --- | --- |
| Cheaper seed preparation versus base | +4.4 [-7.9, +16.8] | Retain tree-identical optimization; no separate Elo claim |
| Quiescence hash ordering versus cheaper seeds | -3.4 [-15.6, +8.8] | Reject |
| Overlay removed versus cheaper seeds | +9.5 [-4.0, +23.0] | Advance to held-out cumulative confirmation |

The quiescence experiment admitted only legal, eligible hash moves, retained SEE
pruning and passed its new picker tests. Nevertheless, it lost 0.3 ply in the
efficiency suite, searched 0.62% more nodes, changed a pinned defensive score,
and broke the null-on/off consistency fixture (`null-in-check`: `e1e2` versus
`d1e2`). It was removed, not accommodated by weakening that contract. Its patch,
match and efficiency reports are archived for future investigation.

The final combined efficiency result is 1.68% fewer depth-eight nodes, +2.76%
fixed-node throughput and no mean completed-depth gain. These trees differ, so
that throughput result is not a pure implementation speedup. The matches are
the relevant strength evidence. Neither retained patch has an independently
confirmed Elo gain, and the current personality cost versus Aggression 0 was
not remeasured.

## Regression records and validation

Only the default-profile search-regression records were re-pinned. Node budgets
and acceptance/endpoint/sacrifice/null-contract inputs are unchanged. Both
engines were probed at 20,000, 100,000 and 400,000 nodes before updating records:

| Record | Old 20k result | New 20k result |
| --- | --- | --- |
| win-hanging-queen | `e1e2`, +659 | `e1e2`, +641 |
| punish-central-queen | `d1d5`, +1457 | `d1d5`, +1446 |
| contain-lone-rook | `a1a2`, -670 | `a1a2`, -647 |
| starting-style | `b1c3`, +22 | `b1c3`, +23 |
| open-game-style | `b1c3`, +21 | `b1c3`, +29 |
| developed-open-game | `f1c4`, +51 | `b1c3`, +52 |

Mate and all tactical/defensive moves remain unchanged. The short development
record now develops the knight; `f1c4` returns on the candidate at both larger
budgets. These probes are engine analysis, not independent certifications of
chess truth. Full observations are in `fitted-regression-probes.json`.

Observed validation:

- `cargo nextest run --no-tests=pass`: **278 passed**;
- the same command with `--features tuning`: **294 passed**;
- `cargo fmt --check` and `cargo clippy --locked --all-targets --features tuning -- -D warnings`: passed;
- locked release build; rebuilt final binary identical to the matched binary;
- endpoint style, sacrifice gates, legacy acceptance, selected-profile-75
  standard acceptance and `validate_acceptance_contract.py`: passed;
- `python3 -m pytest tools/tests/test_splice_weights.py -q`: **9 passed**;
- full compressed-PGN/manifest/hash and pair-statistics audit of the archived
  matches, described below.

The full Python suite was not rerun; its tools were not modified. Earlier
experimental test failures were resolved by removing the quiescence experiment
and updating only the reviewed default evaluation records.

## Provenance, artifacts and reproduction

Measurements used Linux x86_64, Rust/Cargo **1.88.0**, the locked release profile
(fat LTO, one codegen unit), one search thread and 16 MiB hash. The host reported
96 online logical CPUs. Each confirmation used concurrency 24; the two matches
overlapped, and builds/checks shared the host. This was not a dedicated benchmark
machine, which is an additional reason not to overstate the narrow 50 ms result.

| Source | Revision | Binary SHA-256 |
| --- | --- | --- |
| Base | `b80144e3a413a08b9db0d816f11bc75e46e2bdbf` | `05dbdd61f0d682674a706ae0ebddc81a2172c0bd4c3984db2e2fea8d245c31f1` |
| Cheaper seeds | `6131d5d6883d3f08587a790ab81970e776443c20` | `65c558e0921a34e54a2e2a11f1954a72961c7b77c2125d9f5837f2d6602bf645` |
| Final engine | `b6b896133b7e738ba410eda8d9b772d5626f2de9` | `4473dd01f432f40c6d207e44b9b82f5ce7a4069cafc26dc5b00a6b072189060a` |

Dependency revision: `7e93cdea094a50c1574081ceb6e7b269ad0234ee`. The screening
implementation simply returned zero from the old intensity method; the final
cleanup preserved all 90 fixed-node traces against that binary.

All artifacts are under [data/aggression75-fitted-mobility](data/aggression75-fitted-mobility/sha256.json).
Five complete PGNs are stored as deterministic gzip files, alongside untouched
manifests, arbiter output, logs and summaries. Original temporary paths and
omitted revision/build fields remain in the hashed manifests;
`provenance.json` supplies the source/build mapping instead of rewriting them.
No executable binaries are bundled.

The committed held-out book consists of the last 1,536 non-comment records of
`selective-search-confirmation.epd`; SHA-256
`5e11312942869d196476eac6e4f493596f1b2e81f11271cfc4152a163e99a9ec`.
These pairs were excluded from all three screens. Both confirmations use that
book, with the clocked run using its first 1,000 pairs. They are not independent
opening corpora; all positions also descend from the repository's existing
opening families.

Build the base and final engine in separate checkouts, then run:

```sh
cargo build --release --locked --bin jakgro --bin selfplay
python3 tools/run_sprt.py \
  --engine target/release/jakgro --baseline-engine /path/to/base/jakgro \
  --candidate-aggression 75 --baseline-aggression 75 \
  --games 3072 --movetime-ms 50 --concurrency 24 --hash 16 \
  --elo0 0 --elo1 10 \
  --openings docs/tuning/data/aggression75-fitted-mobility/a75-heldout.epd \
  --pgn /path/to/scratch/a75-confirm.pgn
```

For the clocked channel replace `--games 3072 --movetime-ms 50` with
`--games 2000 --time-control 1.0+0.01`. To audit rather than replay games,
decompress each archived PGN into scratch, copy its original manifest beside it,
and run `tools/analyze_match.py`. Recompute the reported paired statistics with
`tools.run_sprt.pair_points_from_pgn` and `tools.run_sprt.evaluate` using
`elo0=0`, `elo1=10`, `alpha=beta=0.05`.

The archived tree-comparison scripts use `JAKKOO_TEMP` as a scratch directory
containing the named rebuilt binaries; run them from the repository root.
They compare all completed UCI iterations after removing only time/NPS fields.

Longer controls, external opponents/books, multi-threaded play, and a new
75-versus-0 personality-cost match remain unverified. They should precede any
broader rating or general-strength claim.
