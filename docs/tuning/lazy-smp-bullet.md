# Eight-thread Lazy SMP: first-pass validation

This series repairs effective helper diversification and removes two classes of
unnecessary work. It does **not** establish a bullet Elo gain or a large NPS
improvement. `Threads` still defaults to one; the thread lifecycle, clock policy,
main worker's root-ordering policy and personality rules are unchanged.

## Changes and build

The cumulative release builds were:

| Label | Source revision | Change from the preceding build |
| --- | --- | --- |
| baseline | `59cd7f2c9247cc4fd4def2a9b9dc57c355de830d` | Starting implementation |
| helpers | `3b4243e1d5686998e8ef0d3ea682ee5a99506b82` | Format UCI PV strings and construct `SearchInfo` only for the main worker |
| tt | `4f40f9b1a882f00f73f6e826ff054696bcbf8be9` | Suppress a TT write when inheriting the recorded move makes the payload equal to the probe snapshot |
| diversified | `37c9c8af9636a0048bcc84c8c89953586e060920` | Rotate helper alternatives after complete root ordering, retaining an available PV/hash move first |

The old input rotation could not affect the final root order: preparation sorted
by score, complexity and the unique move key. The new worker-local rotation
reaches that production sorter. With no preferred move it rotates the entire
order; otherwise it rotates only the alternatives. Rotation zero preserves the
main ordering. Small restricted move sets can necessarily give helpers the same
order, and live TT/history races still make multi-threaded searches nondeterministic.

The TT change compares against the **probe snapshot**, not a freshly loaded slot.
A skipped stale write can leave newer work alone, including work produced by the
same worker's subtree. Consequently, this is consistent with the existing
snapshot policy, but is not a universal promise of identical single-thread trees.

All binaries used Rust 1.93.1 / LLVM 21.1.8, the existing release profile (fat LTO,
one codegen unit, aborting panics), no additional `RUSTFLAGS`, and the locked
`cozy-chess` revision `5851b224ca58ef3330b9d41813fa76f391bb60cf`.
The NNUE file was unchanged across builds; its SHA-256 is
`afbc0897e26c88720d9fe46d5cb90c906512d07eb8e69ba46c74ade4b944350b`.
See [provenance.json](data/lazy-smp-bullet/provenance.json) for binary hashes.
No binaries are bundled with this report.

During the investigation the source checkout advanced to
`e0230ff18f6c403df810f36f20c230de2127043b`, adding separate interior/quiescence
telemetry, different bucket indexing and owned TT mappings. The implementation
series was replayed onto that base. The revisions and binary hashes above remain
the **as-measured, pre-integration** versions; these timing and match estimates
do not measure the newer combination. Build/test integration checks are distinct
from performance or strength confirmation on the newer base.

The host exposed 48 physical cores / 96 logical CPUs on an Intel Xeon Platinum
8488C. Measurements used one logical CPU per physical core, not SMT siblings.
Timing samples were serial on CPUs 0–7; this session ran no match or build
alongside them. Affinity is not exclusive core reservation, however, and
independent repository work landed during this investigation. Background host
load was not fully controlled, so the small timing differences are indicative.
The short matches used disjoint CPU sets 0–15, 16–31 and 32–47, one game at a time
per set. Even counting both configured engines, this session reserved at most
48 engine threads. The same allocation and settings applied to both sides of
each match; this does not establish an otherwise-idle host.

## Correctness and single-thread checks

These checks describe the pre-integration builds measured above.

- The full default suite passed: **374/374 tests**. The focused root/helper rerun
  passed 29 tests, and `cargo +1.93.1 clippy --all-targets -- -D warnings` passed.
- New tests check the effective post-sort order, all seven helper indices,
  preferred moves, unequal-ranked alternatives, restricted/empty/singleton roots,
  wrapped rotations, and move/child association. A low-window root search checks
  the first move actually searched, rather than only testing a utility.
- Helper outcome tests cover both depth schedules, absence of reports/formatted
  info, node publication and table contribution. TT tests cover unchanged
  retained-move payloads, other field changes and an intervening deeper entry.
- The SPRT runner tests check Threads forwarding for clock and movetime limits,
  bounds 1–128, default one-thread behavior, manifest agreement and rejection of
  multi-thread fixed-node comparisons. The related Python suites passed.

For cross-binary checking, the ten positions in `tests/data/search-performance.epd`
were searched at 100,000 nodes and depth eight, twice each, on each of the four
binaries, with one thread, Aggression 75 and NNUE enabled. All **160 observations**
were present; all 80 repeat pairs reproduced. Best move, score, completed depth,
reported nodes and PV matched across the four builds in every tested case.
This is evidence for these fixtures and settings, not all positions or profiles.

## Paired timing screen

The same suite used 12 measured samples per position, binary, thread count and
movetime, plus warm-ups: **1,920 timed observations** at 1/8 threads and 50/250 ms.
Hash was 16 MiB and Aggression 75. Every sample began with `ucinewgame`; binary
order was rotated/reversed between samples and positions.

The following are geometric ratios of per-position median **reported** NPS.
Each incremental row compares against its predecessor, not the original baseline:

| Comparison, eight threads | 50 ms | 250 ms |
| --- | ---: | ---: |
| helpers / baseline | +0.00% | −0.35% |
| tt / helpers | −0.12% | +0.69% |
| diversified / tt | +0.68% | −0.17% |
| diversified / baseline | **+0.56%** | **+0.17%** |

The complete candidate's fixture-bootstrap 95% intervals were approximately
[+0.01%, +1.15%] at 50 ms and [−0.60%, +0.80%] at 250 ms. These resample just ten
fixture medians, not hardware, games, or independently repeated experiments.
The small and inconsistent incremental differences should not be promoted to
independent speed claims. Mean median completed-depth differences for the full
candidate were +0.05 and +0.15 plies, respectively; neither is an Elo result.

UCI node/time counters describe the last completed main iteration. They omit
later abandoned-iteration work and do not measure the helper-join tail. The
harness therefore also timed `go` submission through receipt of `bestmove`.
At eight threads the complete candidate's p95 wall latencies were 50.44 and
250.98 ms; observed maxima were 51.87 and 251.59 ms. This includes client
reader/consumer scheduling and is not a hard real-time guarantee. Configuration,
`ucinewgame`, and position setup are outside that wall interval.

Median first-info latency was around 4 ms with eight threads versus roughly
0.5 ms with one. That combines launch/allocation, first-iteration work and UCI
scheduling; it is not an isolated measurement of thread-creation cost. It makes
worker-buffer reuse a sensible next profiling target, rather than evidence for
an immediate thread-pool rewrite.

Raw samples and all per-fixture medians, intervals and latency distributions are
in [timing.json.gz](data/lazy-smp-bullet/timing.json.gz) and the readable
[timing-summary.json](data/lazy-smp-bullet/timing-summary.json).

## Accelerated equal-clock screens

Both sides used eight threads, Hash 16 MiB, Aggression 75 and the same explicitly
loaded NNUE file. Each opening was played with reversed colors. The clock was
**one second plus 10 ms per move**, not one minute. The archived launcher recorded
an explicit **10-ms time-forfeit grace**, rather than silently using selfplay's
250-ms default.

| Incremental comparison | Games | Candidate score | Approximate paired Elo estimate (95% normal interval) | LLR, H0=0 / H1=10 |
| --- | ---: | ---: | --- | ---: |
| helpers / baseline | 128 | 50.78% | +5.4 [−20.8, +31.8] | 0.024 |
| tt / helpers | 128 | 54.30% | +29.9 [+5.2, +54.9] | 1.573 |
| diversified / tt | 256 | 52.54% | +17.7 [+1.3, +34.1] | 1.807 |

All **512 games completed without recorded faults**. PGN hashes, completed game
counts and color-reversed opening pairs passed `analyze_match.py` validation.
Every sequential-test verdict remained **continue**; none crossed the +2.944
acceptance boundary. The analyzer's more conservative Hoeffding intervals are
also archived and are wider than the normal intervals above.

These screens are encouraging, particularly for the two search changes, but do
not establish an accepted Elo gain. Do not add their point estimates together or
transfer them to one-minute bullet. A longer, predeclared confirmation remains
necessary for a strength claim.

## One-minute bullet smoke check

The complete candidate also played the original baseline at **60 seconds plus
100 ms per move**, with eight threads on each side, the same 16-MiB Hash,
Aggression 75, NNUE file and 10-ms grace. This used two simultaneous games on
CPUs 0–31 and a separate twelve-opening slice: non-comment book entries 256–267
(zero-based), outside the first 128 entries used by the short screens.

All **24 games completed without recorded faults**: **10 wins, 6 draws and
8 losses** for the candidate, or **54.17%**. PGN integrity and reversed-color
pairing passed the same analyzer. The approximate paired estimate was +29.0 Elo
with a 95% normal interval of [−9.3, +68.1]; LLR was 0.628 and the verdict remained
**continue**. This is a one-minute lifecycle/playing smoke check, not a strength
acceptance test. It cannot establish a bullet Elo improvement.

The opening slice, compressed PGN and all manifests/statistical results are in
the archive under `bullet-*` / `bullet.*`. To repeat this layout with the fresh
match command below, select `--time-control 60+0.1 --games 24`,
`--openings "$R/bullet-openings.epd"`, `--concurrency 2` and CPUs 0–31.

## Integration checks on the newer TT base

After replay onto `e0230ff18f6c403df810f36f20c230de2127043b`, the combined
implementation at `69e7eb7ae8531bbca4f4712a6b8b58e4742e983f` passed **380/380
release-profile tests**, including the new table-mapping tests and the SMP
regressions. Formatting and the release build passed as well. The integrated
binary hash and commands are recorded in
[integration.json](data/lazy-smp-bullet/integration.json).

The conventional `cargo clippy --all-targets` check completed with one warning
already present in the newer source base: `manual_slice_size_calculation` in
`bucket_allocation_uses_the_requested_size`. Adding `-D warnings` therefore
failed on that pre-existing test expression. It was left unchanged rather than
folding an unrelated lint cleanup into these SMP changes; the diagnostic is
preserved in the integration check log.

These checks establish build/test compatibility, not performance or strength
of the combined implementation. Timing and matches were not repeated after
integration, and the earlier normal confidence intervals do not account for
uncontrolled background load.

## Reproducing and auditing

The archive preserves the as-run scripts, input/binary hashes, test logs,
compressed PGNs, arbiter results, manifests and both statistical summaries.
Recorded commands retain their original temporary build paths; supply new paths
to reproduce a run. Build with the compiler/profile/network above, for example:

```sh
cargo +1.93.1 build --release --locked --bin jakgro --bin selfplay
```

With four saved cumulative binaries, reproduce the timing layout from the
workspace root:

```sh
R=docs/tuning/data/lazy-smp-bullet
taskset -c 0-7 python3 "$R/measure.py" \
  --variant baseline=/path/to/baseline \
  --variant helpers=/path/to/helpers \
  --variant tt=/path/to/tt \
  --variant diversified=/path/to/diversified \
  --threads 1 8 --times-ms 50 250 --samples 12 \
  --suite "$R/performance.epd" \
  --aggression 75 --hash 16 --json /tmp/lazy-smp-timing.json
```

Do not use `--skip-fixed` when claiming invariance: the as-run script emits empty
comparison lists for that mode. The archive validator requires the full fixed
observation grid and checks its signatures directly; absence of samples cannot
pass as equality.

For a fresh paired match with an explicit grace:

```sh
R=docs/tuning/data/lazy-smp-bullet
taskset -c 0-15 python3 "$R/match.py" \
  --runner /path/to/selfplay \
  --engine /path/to/candidate --baseline-engine /path/to/baseline \
  --candidate-name Candidate-8 --baseline-name Baseline-8 \
  --candidate-aggression 75 --baseline-aggression 75 \
  --candidate-eval-file "$PWD/nets/jakgro.nnue" \
  --baseline-eval-file "$PWD/nets/jakgro.nnue" \
  --threads 8 --hash 16 --concurrency 1 \
  --games 256 --time-control 1+0.01 --time-grace-ms 10 \
  --openings docs/tuning/data/selective-search-confirmation.epd \
  --pgn /tmp/lazy-smp-fresh.pgn
```

The launcher refuses to overwrite existing evidence. For historical PGN analysis,
extract the relevant `.pgn.gz` and use `tools/analyze_match.py` with its archived
manifest; do not invoke `run_sprt.py` on the existing PGN. The archive can also be
checked without running any engine:

```sh
python3 docs/tuning/data/lazy-smp-bullet/validate.py
```

The audit checks data integrity and reported statistics, not a rerun of the
matches or a rebuild of the recorded executables. Persistent workers, retained
helper histories, NUMA placement and TT replacement-policy retuning remain
outside this series.
