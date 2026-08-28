# Second measured strength series

## Verdict

Two changes landed out of four attempted. Against the first series' head at a
fixed 50 ms per move over 4096 colour-reversed games, the result is **+65.5 Elo
[57.9, 73.1] at Aggression 75** and **+32.0 Elo [24.7, 39.3] at Aggression 0**.
A clocked channel at `1.0+0.01` over 1200 games measures **+50.4 Elo [36.6,
64.4]**, with no time forfeits and no protocol faults. All three cross the
predeclared `elo0=0, elo1=20, alpha=beta=0.05` H1 boundary.

The default profile's personality strengthened rather than eroded. Over 600
fixed-node games against objective play, forcing-move retention rises from 100.2%
to **110.8%** against a 90% floor, checks per hundred moves from 11.19 to 13.11
against 8.28 for objective play, and the measured cost of the default profile
narrows from -115 Elo to **-65 Elo**.

## Calibration

Before starting, the engine was measured against itself at half time to learn what
throughput is worth here: **a 2x speedup is +114.8 Elo [104.0, 125.8]** at
Aggression 75, so

> Elo ≈ 115 × log₂(speedup)

That slope is steep, which is what made throughput work worth attempting at all,
and it is how each patch's expected value was estimated before measuring it. The
figure is an upper-ish estimate: the handicapped side ran behind a Python wrapper
that adds pipe latency, and the slope always flattens at longer time controls.

## Provenance

| Input | Value |
| --- | --- |
| Baseline revision | `0e72ff8cd572fe5fec799db5ce8450fa01f04da3` |
| Baseline binary SHA-256 | `026978565cf8607d…` |
| Candidate binary SHA-256 | `a22059fb224a43a6…` |
| Dependency revision | `7e93cdea094a50c1574081ceb6e7b269ad0234ee` |
| Opening corpus | `data/selective-search-confirmation.epd`, 2048 positions |
| Artifacts | `data/strength-series-two-a75.sprt.json`, `data/strength-series-two-a0.sprt.json`, `data/strength-series-two-a75-clocked.sprt.json` |

## What landed

| Patch | Aggression 75 | Aggression 0 | Mechanism |
| --- | --- | --- | --- |
| Pawn and king structure cache | +6.4 [0.7, 12.1] | not measured | +2.5% throughput, tree identical |
| Quiescence transposition access | +44.8 [36.6, 53.2] | +35.7 [27.4, 44.0] | 22.4% fewer nodes, 1.28x faster |

Cumulative search behaviour over the frozen `tests/data/search-performance.epd`
suite: 22.36% fewer nodes and +0.500 ply at Aggression 75, 14.74% fewer and
+0.200 ply at Aggression 0.

## What did not land, and why

Three of the four planned patches were implemented, measured, and reverted. Each
is recorded with the numbers that rejected it, so a later attempt starts from
evidence rather than repeating the experiment.

**Compact transposition entries, 24 to 16 bytes.** Correct and fully tested, but
1.4% slower at Aggression 75 with byte-identical node counts. The measurement that
explains it also redirected the series: at depth nine, 15,953,851 of 16,365,809
nodes are quiescence, leaving 411,958 interior nodes that probe the table roughly
once each. Only about 384,000 probes happen in a 16-million-node search, so
capacity was never the constraint and doubling the entries per mebibyte changed
nothing while the extra packing cost a little. That reading is what produced the
quiescence patch, which is worth an order of magnitude more.

**Internal iterative reduction.** Reducing depth by one at non-principal-variation
nodes with no hash move cut 23.6% of nodes and gained 0.3 ply, and still measured
**-5.1 Elo [-11.6, +1.4]** at Aggression 75 at a threshold of depth four, and
+2.5 [-2.2, +7.3] at depth seven. The node saving comes from searching a genuinely
shallower tree rather than from better ordering. A plausible reason this engine
differs from convention: now that quiescence stores entries, hash moves are far
more available, so nodes without one are the unusual ones and reducing them is
closer to pure depth loss.

**Singular extensions.** Implemented completely, including a `SearchMode::Exclusion`
that neither reads nor writes the table. Reading had to be disabled as well as
writing, or the exclusion search hits the very entry it is testing and takes a
cutoff without ever searching the alternatives; that bug was visible as a zero
extension rate with one node per probe. Once working, 3.6% to 25.2% of tested hash
moves proved singular, but exclusion searches cost 10.7% to 30.6% of all nodes.
Four configurations were measured and every interval included zero: the best was
+6.9 [-0.4, +14.3] at Aggression 75. The probe is expensive here precisely because
quiescence dominates, so a reduced re-search at the same ply re-expands a large
quiescence subtree.

## Limitations

- The rejected patches were measured at 40 to 60 ms per move. Extensions in
  particular usually pay more at longer controls than this host can run.
- The quiescence entries share the table with interior entries and claim depth
  zero, so in principle they could evict deeper results. Instrumenting the
  replacement path shows they effectively never do: across three depth-ten
  searches totalling 28 million nodes and 8.8 million replacements, a depth-zero
  entry displaced a deeper one twice, a rate of 0.00%. The existing policy already
  prefers replacing shallow and stale entries, so a separate table or a
  depth-aware eviction rule has almost no headroom to recover.
- Selecting a sacrifice fixture surfaced a pre-existing defect: for some
  positions the engine emits a `bestmove` with no `info` line, which the
  measurement tools cannot read. It reproduces on the previous release build.
- The cache is thread-local and verified by full key, so it is correct under a
  future parallel search, but no parallel search exists to confirm that.

## Reproduction

```sh
cargo build --release --locked --bin jakgro --bin selfplay
python3 tools/run_sprt.py \
  --engine target/release/jakgro \
  --baseline-engine /path/to/baseline/jakgro \
  --candidate-aggression 75 --baseline-aggression 75 \
  --games 4096 --movetime-ms 50 --concurrency 44 \
  --elo0 0 --elo1 20 \
  --openings docs/tuning/data/selective-search-confirmation.epd \
  --pgn artifacts/strength-series-two-a75.pgn \
  --summary-json artifacts/strength-series-two-a75.sprt.json
```

Swap both aggression flags to `0` for the objective channel, or replace
`--movetime-ms 50` with `--time-control 1.0+0.01` for the clocked confirmation.
The deterministic gates are unchanged:

```sh
python3 tools/measure_style.py --engine target/release/jakgro --check
python3 tools/measure_style.py --engine target/release/jakgro \
  --suite tests/data/sacrifice-gates.epd --check
python3 tools/measure_acceptance.py --engine target/release/jakgro --check
python3 tools/validate_acceptance_contract.py
cargo nextest run --locked
```
