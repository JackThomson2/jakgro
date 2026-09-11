# Follow-up screens and the 200 ms quiet-check check

## Verdict: no additional playing change

Two more candidates were tested against the accepted engine at `b913f6c`.
Neither earned an additional engine patch. Both implementations were restored
to the base, and this commit contains only the report and its evidence.

| New candidate, Aggression 75 versus the same base profile | Games at 50 ms | Elo, paired normal 95% interval | Decision |
| --- | ---: | --- | --- |
| Avoid unused root tactical/conversion analysis | 1,024 | **-10.9 [-23.5, +1.7]** | Reject: no demonstrated strength gain; [0, 10] test accepts H0 |
| Apply ordinary root risk guards consistently | 1,024 | **+1.0 [-10.7, +12.7]** | Reject: inconclusive strength and lost default investment target |

The intervals do not establish that either candidate is strictly weaker. They
also do not establish an improvement. H0 in the first screen is not a proof of
a particular negative Elo value. No acceptance fixture was changed this round.

## The previously accepted change at a longer search time

A separate match compared the **already accepted quiet-check change** with its
immediate predecessor, using the same immutable binary pair as the previous
series. This is not a comparison of either new candidate above, and not a gain
to add to the earlier +44.9/+39.6 estimates.

| Setting | Result |
| --- | --- |
| Aggression | 75 on both sides |
| Limit | 200 ms/move, four times the earlier 50 ms limit |
| Games / pairs | 1,536 / 768, colors reversed |
| W / D / L | 576 / 509 / 451 |
| Score | 54.069% |
| Relative Elo, paired normal 95% interval | **+28.3 [+19.4, +37.3]** |
| [0, 10] LLR | 11.280, accept H1 |
| Engine faults | None |

The conservative Hoeffding interval from `analyze_match.py` is **[-5.8, +63.0]**,
which still crosses zero. Both statistical reports are archived. The LLR was
evaluated at the preselected game cap, not used to stop the run early.

Style remains similar in this comparison: 30.584 forcing moves per hundred
against 30.433 (**100.5% retention**), and 10.029 checks against 10.096 (**99.3%
retention**). These SAN counts are descriptive proxies, not move-quality labels.
The result supports the accepted change at a longer search time; 200 ms is still
far from tournament time controls.

## Rejected experiment: avoid unused root analysis

Three pieces of work were removed without intending to change node-limited
search choices:

- Construct the immediate-child tactical snapshot in `sacrifice_profile` only
  when the PV lacks a legal reply. Legal-reply branches consume a different
  snapshot and never read the immediate one.
- Compute root draw/simplification metadata only when the existing selection
  predicate can use it: aggression at least 75 and conventional score at least
  200 cp. A shared predicate tied construction and selection to the same rule.
- Read material balance directly when detecting a sterile major-piece exchange,
  instead of extracting every style feature just to read material balance.

All **90** fixed-node comparisons at 0/75/100 preserved the completed-iteration
scores, nodes, PVs, best moves and personality counters. The ten depth-eight
performance trees were identical too. The repeated efficiency run measured
**+5.56% throughput**, with no mean completed-depth gain. Its 282 Rust tests,
298 tuning-feature tests, formatting, Clippy and style/acceptance checks passed.
A reference test compared every sacrifice-profile field with the eager
implementation, including accepted, declined, unverified and non-offers.

That was not enough: the strength screen scored below the base. Timed searches
can finish different iterations and personality-verification stages even when
fixed-node trees agree. This run does not identify the cause of its result, but
it does not justify presenting the speed measurement as Elo. The optimization
and its tests are archived as `root-analysis-rejected.patch`, not shipped.

## Rejected experiment: consistent ordinary risk guards

This candidate combined two policy changes at 75:

- apply the existing score-loss penalty to all non-verified offers, rather than
  exempting moves whose initial offered material exceeds the sacrifice threshold;
- keep ordinary moves capped at 16 cp in winning positions instead of allowing
  the winning-position branch to increase their margin to 20. Verified
  investments would retain their 67-to-20 cp conversion rule.

Its new policy tests and all 282 Rust tests passed. The legacy endpoint and
sacrifice checks passed. However, the **default-profile** acceptance suite no
longer chose `e5c6` in `standard-knight-c6-investment`: it chose the objective
`d2d4` instead. This is a lost attacking investment preference, not a newly
illegal move or a proven tactical blunder. The strength screen was essentially
neutral, so there was no demonstrated gain to justify that style trade.

The two changes were tested together, not separately attributed. The existing
rules remain unchanged. `root-policy-rejected.patch` and the failed acceptance
report are retained so a future investigation can distinguish this experiment
from a claim that the guards have been fixed or individually measured.

## Provenance

Both rejected patches apply independently to `b913f6c58edbe53b9129c090c0bc39bae1052d2f`.
They are alternatives, not a cumulative series. Their games used the first 512
positions of `selective-search-confirmation.epd`, concurrency 20, one search
thread and 16 MiB hash.

| Binary | SHA-256 |
| --- | --- |
| Rebuilt current base | `8558c2e87f5249d4f2f211763630410f40b7e95c973f25582fa6a6793996690e` |
| Root-analysis candidate | `bef3e069a79f81c53a27ff3ca59955c7559c4c91a1fa496160e9a5546ff1edba` |
| Root-policy candidate | `c03f08e926ada46ce14145fb24bfc46979410cff2191d2d095d52d60af0c38c6` |
| Accepted quiet-check binary used at 200 ms | `761fbb44921e9bb7eff967b4aae603f47c9c7871297229848e9cc9b062c7a009` |
| Its pre-quiet-check predecessor | `aab08df1d6e479010d68d8feba5a604db58393acf7d202868ad41e8e46fc497f` |

The 200 ms pair maps to engine sources `c957c4d` (accepted as `b913f6c`) and
`a0d24fb`, as recorded in the quiet-check series. It used concurrency 24 and the
first 768 positions of the 1,536-position `a75-heldout.epd` already committed
under `data/aggression75-fitted-mobility`. These positions were previously
measured; this was not a fresh external book or a new family of openings.

Builds used Rust/Cargo 1.88.0, the locked release profile and dependency revision
`7e93cdea094a50c1574081ceb6e7b269ad0234ee`. The Linux x86_64 host reported 96 online
logical CPUs. The 200 ms match overlapped with the shorter screens, builds and
checks. These are shared-host measurements, not dedicated-machine benchmarks.

## Archive and reproduction

The [artifact index](data/aggression75-followup-screens/sha256.json) binds the
three complete compressed PGNs, original manifests, arbiter outputs, logs,
statistical/style reports, rejected patches, tests, fixed-node comparisons and
acceptance outcomes. No executable binaries are bundled.

To repeat the longer-control check, rebuild the two named quiet-check-series
versions separately, then run:

```sh
cargo build --release --locked --bin selfplay
python3 tools/run_sprt.py \
  --engine /path/to/accepted-quiet-check/jakgro \
  --baseline-engine /path/to/pre-quiet-check/jakgro \
  --runner target/release/selfplay \
  --candidate-aggression 75 --baseline-aggression 75 \
  --games 1536 --movetime-ms 200 --concurrency 24 --hash 16 \
  --elo0 0 --elo1 10 \
  --openings docs/tuning/data/aggression75-fitted-mobility/a75-heldout.epd \
  --pgn /path/to/scratch/quiet-check-200ms.pgn
```

To audit rather than replay, decompress each PGN into scratch with its original
manifest beside it. Run `tools/analyze_match.py`; recompute the paired-normal
result with `tools.run_sprt.pair_points_from_pgn` and `tools.run_sprt.evaluate`
using 0, 10, 0.05, 0.05. The archive audit covered **31 indexed files and all
3,584 games**, as well as the root-analysis tree equality and the failed
investment target. Playing sources and fixture files are unchanged.

The next substantial avenue is a separately validated objective-evaluation
refit using stronger/longer-search games and a genuinely separate opening set.
That is a research direction, not an unmeasured gain promised by this report.
