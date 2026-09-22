# NNUE at the default Aggression 75

This series replaces the handcrafted static evaluation with a quantized
network trained on the engine's own fixed-node self-play and makes it the
shipped default. Every figure below is a fixed-node or 50 ms/move self-play
result, against the frozen handcrafted engine at commit `5c242b7` and measured
by `./autoresearch.sh` unless a row says otherwise; none is an absolute rating.

## Measurement

- Candidate: the current build with the freshly trained network loaded through
  `EvalFile`, Aggression 75. Baseline: `5c242b7` (handcrafted evaluation),
  Aggression 75.
- Strength: 2048 paired games at 50,000 nodes per move, one thread, over
  `docs/tuning/data/selective-search-confirmation.epd`, evaluated by
  `tools/run_sprt.py` (pentanomial pair statistics). The match is byte-for-byte
  reproducible.
- Style: the forcing-move ratio of Aggression 75 over Aggression 0 in a
  512-game match with the same network on both sides, compared with the same
  ratio for the handcrafted engine (1.062).
- Throughput: median fixed-node nodes per second on
  `tests/data/search-performance.epd`, candidate over baseline.

## Results

| Run | Change | Elo vs HCE-75 (95% CI) | Style ratio | NPS ratio |
| --- | --- | --- | --- | --- |
| 1 | Refit-pilot corpus (140k rows), λ 0.5, 10 epochs | -376 [-397, -358] | 1.062 | 1.20 |
| 2 | λ 0 (teacher-only labels) | -315 [-333, -298] | 1.065 | 1.20 |
| 5 | 1.45M self-play rows, weights clamped to export bounds | -137 [-149, -126] | 1.099 | 1.16 |
| 6 | 30 epochs, epoch chosen by development label loss | -114 [-125, -103] | 1.075 | 1.10 |
| 7 | All-Rust pipeline, 2.74M rows | -73 [-84, -63] | 1.041 | 1.12 |
| 9 | Learning-rate decay 0.9 per epoch | -51 [-61, -41] | 1.055 | 1.17 |
| 12 | 5.8M rows | +2 [-7, +12] | 1.067 | 1.20 |
| 14 | AVX2 kernels (same network) | +12 [+2, +22] | 1.063 | 1.28 |
| 18 | 11.7M rows | +63 [+53, +73] | 1.076 | 1.33 |
| 19 | +2.8M rows taught by the run-18 network | +79 [+69, +89] | 1.058 | 1.36 |
| 21 | Horizontal king mirroring, eight buckets | +98 [+87, +109] | 1.097 | 1.29 |
| 22 | 17.3M rows | +115 [+105, +126] | 1.073 | 1.26 |
| 26 | 22.9M rows (64 handcrafted-taught, 64 network-taught seed groups) | +123 [+112, +134] | 1.057 | 1.30 |
| 31 | 25.8M rows incl. 16 groups taught at 100k nodes; batch 8192, rate 0.0057 | +136 [+125, +147] | 1.052* | 1.32 |
| 33 | Eight piece-count output buckets (format v2) | +130 [+119, +141] | 1.114 | — |
| 36 | 40 epochs | +138 [+127, +149] | 1.094 | — |
| 38 | 31.4M rows (176 seed groups, 32 taught at 100k nodes) | +142 [+131, +153] | 1.083 | 1.24 |
| 42 | 34.2M rows (192 seed groups, 48 taught at 100k nodes) | **+147 [+136, +158]** | 1.077 | 1.28 |
| n512 | 512 squared clipped-ReLU units (format v3), 131.0M rows of 10k-node self-play taught by run 42, λ 0.25, 80 epochs | +59.2 [+52.6, +65.8] over run 42 at 50 ms/move† | — | — |

\* re-measured over 2048 games; the 512-game figure was 1.009.

† measured against the run-42 network under the clock, not against the
handcrafted baseline at fixed nodes like the rows above; no
handcrafted-baseline, style-ratio or throughput figure has been measured for
this network.

The run-42 network (SHA-256
`b544ba91fd4caf068a544864047fedcbdae98b6aeea37b6edebb0e485b298eca`) was the
last 128-unit clipped-ReLU network. At 50 ms per move over 1024 games it scored
71.8%, +163 Elo [146, 180], against the same baseline (run 26 scored +143
[127, 160], run 38 +160 [145, 176] under the same clock).

The published network (`nets/jakgro.nnue`, SHA-256
`30aa7dc648107a17340fd5789d2825eeb01be3ba631a4ad0c8becb0f1e341151`) is the
512-unit squared clipped-ReLU network described below (format version 3,
6,308,944 bytes). It was trained on 131.0M rows (140.6M raw) of 10,000-node
self-play by the engine at `da2cc416` with its embedded run-42 network as
player and teacher, Aggression 75 against 0 from random-ply prefixes of 8, 12
and 16 plies; the development split is 3.32M rows from three held-out seed
groups, one per prefix length. Training ran for 80 epochs at batch 8192, rate
0.0057 decaying by 0.95 per epoch, L2 1e-6, seed 75 and λ 0.25; epoch 79 was
selected at a development label MSE of 0.009536 (a λ 0.25 target, not
comparable with λ 0 figures) and an outcome MSE of 0.06897. Against the run-42
network at Aggression 75 it scored +59.2 Elo [52.6, 65.8] over 4096 games at
50 ms per move on an otherwise idle host. The fixed-node chain that selected
it, at 50,000 nodes per move on the same prepared data: a λ 0, 30-epoch,
decay-0.9 network measured +29.8 [21.3, 38.2] over run 42 in 2048 paired
games; its λ 0.25 sibling measured +17.7 over that network; and the published
80-epoch schedule measured +18.7 [12.6, 24.8] over the λ 0.25, 30-epoch
sibling.

The network is regenerated from the repository as follows. Build the
`da2cc416` engine and the tuning binaries (`cargo build --release --locked
--features tuning --bin jakgro --bin selfplay --bin tune --bin nnue-data`),
then play 120 corpus groups, group `g` in `0..=119` using the eight seeds
`1000 + 8g` to `1000 + 8g + 7` and a random-ply prefix of `8 + 4 * (g % 3)`:

```sh
python3 tools/generate_nnue_corpus.py --engine <da2cc416 jakgro> \
  --runner target/release/selfplay --tune target/release/tune \
  --openings docs/tuning/data/selective-search-confirmation.epd \
  --games-per-seed 4096 --seeds <1000+8g .. 1000+8g+7> --nodes 10000 \
  --random-plies <8|12|16> --concurrency 64 --output corpus/g<g>.txt
```

Groups 0 to 116 concatenated are the training corpus and groups 117, 118 and
119 the development corpus; then

```sh
./target/release/nnue-data prepare --training training.txt \
  --development development.txt --output-dir data \
  --deduplicate --drop-development-overlap
./target/release/nnue-data train --data-dir data --output-dir net \
  --epochs 80 --batch-size 8192 --rate 0.0057 --rate-decay 0.95 \
  --l2 1e-6 --seed 75 --lambda 0.25
```

The trainer's output does not depend on `--threads`. The published run
prepared its data with the 128-unit-era helper, whose files carry a version 1
header; the current helper writes the same rows under the version 2 header,
which the trainer accepts alike.

The output buckets did not move the fixed-node result but lowered the
development label loss by 2% and were kept; the piece-count bucket lets the
network value the same material differently in middlegame and ending.

Rejected: L2 1e-4 (-43 Elo), λ 0.1 (within noise), a 256-unit hidden layer at
5.8M and at 20M rows (within noise at fixed nodes, and 20% fewer nodes per
second) and again at 31.4M rows with output buckets (+138 fixed-node, +156
under the clock against +142/+160 for 128 units, 10% fewer nodes per second),
sigmoid scale K 1.2 (-14) and 0.7 (within noise), λ 0.1 at 31M rows (within
noise), 45-, 50- and
60-epoch schedules (within noise), learning rate 0.008 at batch 8192 (within
noise, worse label loss), and damping the network's static score toward the
fifty-move draw (+6 within noise; it flipped a knife-edge sacrifice fixture).

## What changed in the engine

- `nets/jakgro.nnue` is embedded and enabled by default; `Use NNUE false`
  selects the handcrafted evaluator and `EvalFile <embedded>` restores the
  built-in network.
- Features are colored piece-square pairs under eight 2x2 king buckets on a
  file-mirrored half board (6144 inputs), so a position and its horizontal
  reflection share weights. One of eight output layers is selected by the
  number of pieces on the board (`(pieces - 1) / 4`). Network files are format
  version 3: a 512-unit feature transformer, squared clipped-ReLU activations
  (`clamp(sum, 0, 255)^2`), output weights quantized by 64 and bounded to
  `-127..=127`, an `i32` output bias in units of `64 * 255 * 255`, and a score
  of `(numerator * 400) / (64 * 255 * 255)` truncated toward zero; the file is
  6,308,944 bytes. Version 2 files (128 units, clipped ReLU) are rejected.
- Prepared datasets name only the feature contract (`# jakgro-nnue-data-v2`,
  6144 features, feature set 1); version 1 headers, which also recorded the
  writer's hidden width, activation ceiling and output scale, still load, since
  rows never depended on those fields.
- Iterative deepening stops on a mate score only once the depth covers the
  mate distance. A shallow iteration could inherit a longer mate from the
  table, and replaying it shuffled won queen endings into a threefold
  repetition; the network shipped at the time then mated KQ v K from
  `4k3/8/8/8/8/8/3Q4/4K3` in 15 moves at 50 ms per move where it previously
  drew. The published network mates it in 7 moves and KR v K from
  `4k3/8/8/8/8/8/3R4/4K3` in 11 to 12 moves under the same clock (two games
  each; clocked games are not bit-reproducible).
- The update and output kernels dispatch to AVX2 at runtime when the CPU
  supports it; the arithmetic is unchanged. The output kernel squares each
  clipped activation, forms the 16-bit product `weight * activation`, and sums
  products in 64-unit `i32` chunks into an `i64`; the weight bound makes every
  step exact by construction.
- Accumulators are 16-bit and every weight row is 64-byte aligned (1 KiB per
  row at 512 units; 6 MiB of feature-transformer weights). The loader proves
  no placement can overflow them (each unit's bias plus, per square, its
  extreme weight over the twelve planes) and rejects a file it cannot prove;
  the run-42 network's bound was [-9636, 8340]. A perspective's update is one
  fused pass (`source + adds - subs`, at most two of each per pass), selected
  once per perspective rather than once per feature.
- Each ply's state starts from whichever of itself and the state one ply up
  shares more king views with the position and then differs in fewer features
  (3.4 changed features per evaluation against 5.2 from the same ply alone).
  Changed squares come from two masks over the board's own eight bitboards. A
  perspective whose view neither state shares starts from the sums last seen
  in that view when seventeen or more pieces remain, and is rebuilt otherwise.
  Scores are bit-identical, so fixed-node trees are unchanged: median
  fixed-node throughput on `tests/data/search-performance.epd` went from 3.06M
  to 4.00M nodes per second on an Apple M2 Pro (1.30 times). Summed search time
  at one million nodes per position fell 1.27 times on that suite and 1.17
  times on eight bare endgames.
- Tried and dropped: prefetching weight rows inside the evaluation (-1%; with
  the whole table aliased into the first-level cache the ceiling was 2-4%), the
  view cache without the piece-count gate (-3% in endgames), a grandparent as a
  third starting state (3.2 against 3.4 changed features). An explicit 256-bit
  `madd` output kernel was slower than the compiler's 128-bit one under
  Rosetta's AVX2 translation and is untested on AVX2 hardware.

## Pipeline

`tools/generate_nnue_corpus.py` plays deterministic fixed-node self-play
between Aggression 75 and 0 from seeded random-ply openings and extracts
`FEN;outcome;score` rows; `nnue-data prepare` computes features, removes
duplicates and development rows that repeat a training position up to colour
and rank mirroring, and binds everything by SHA-256 tree digests; `nnue-data
train` recomputes every row's features from its FEN, then runs float32 Adam
over sixteen persistent gradient-shard workers (thread-count independent),
flushes subnormal moments, clamps weights to the export bounds, exports every
epoch and selects the lowest development label loss. `tools/nnue_recipe.sh`
records the 128-unit era's corpus (192 seed groups of 4096 games: 64 taught by
the handcrafted engine at 50k nodes, then 32, 16, 16 and 16 taught at 50k
nodes and 16, 16 and 16 at 100k nodes by successively stronger published
networks) and training settings (40 epochs, batch 8192, rate 0.0057 decaying
by 0.9 per epoch, λ 0, K 0.88); a whole run (prepare, train, match, style,
throughput) took about ten minutes on 96 cores, with corpus members cached per
seed group. The published 512-unit network's corpus and schedule are the ones
given under Results.

## Style

The fixed-node fixtures in `tests/data` pinned handcrafted-era outputs and were
re-pinned to the network; each published network moves the 20,000-node
knife-edge positions again, so the profile-control fixtures are mined from
self-play against the shipped network: positions where Aggression 100 invests
material and Aggression 0 does not (`rook-b7-investment`, a rook for a pawn
that costs 56 centipawns under the objective search; `knight-e5-investment`,
a knight for two pawns), declines an unsound king-side sacrifice with the same
move as Aggression 0 (`unsupported-bishop-f6`, `unsupported-rook-f7`,
`unsupported-knight-g7`), keeps the queens on where Aggression 0 trades them
(`avoid-queen-trade-bishop-d4`), and storms or thrusts a pawn where Aggression
0 moves a piece (`opposite-castle-storm`, `queenside-pawn-thrust`) or chooses
a different central push (`central-pawn-thrust`). The standard-profile
acceptance suite has two sacrifices that Aggression 75 makes within its
ceiling (`standard-knight-e4-investment`, a knight for a pawn at 18
centipawns; `standard-knight-d5-investment`, at 112). The verified-null
contract allows a one-pawn score drift between the pruned and unpruned
searches, and its in-check position has a single winning capture, which the
network values at about +800.

The 75-over-0 forcing-move ratio of the run-42 network was 1.08 over 512 games
(handcrafted 1.06), and its forcing-move rate against the handcrafted engine
1.15 times the handcrafted engine's own; the published network's style has not
been measured.

## Limitations

- Every strength figure is against one frozen binary at short controls: the
  handcrafted engine for the 128-unit runs, the run-42 network (and its
  512-unit siblings) for the published network. No absolute rating or
  long-time-control result is claimed.
- The corpus lives under `artifacts/` and is regenerated deterministically by
  the recipe rather than stored in the repository; the 128-unit era's teacher
  networks are kept under `artifacts/autoresearch/teachers`, and the published
  network's teacher is the run-42 network embedded in `da2cc416`.
- The 256-unit network was not adopted at 34.2M rows because its fixed-node
  gain was within noise and it cost about 20% of throughput. The 512-unit
  network's gain over run 42 is measured under the clock, so it includes its
  throughput cost; that cost has not been measured separately.
- Data returns were small at 128 units: 22.9M to 34.2M rows moved the
  fixed-node result from +123 to +147, about +5 per 3M rows, all from groups
  labelled at 100k nodes. The remaining levers are deeper teacher labels and,
  under a clock, throughput.
