# NNUE at the default Aggression 75

This series replaces the handcrafted static evaluation with a quantized
network trained on the engine's own fixed-node self-play and makes it the
shipped default. Every figure below is a fixed-node or 50 ms/move self-play
result against the frozen handcrafted engine at commit `5c242b7`, measured by
`./autoresearch.sh`; none is an absolute rating.

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

\* re-measured over 2048 games; the 512-game figure was 1.009.

The published network (`nets/jakgro.nnue`, SHA-256
`b544ba91fd4caf068a544864047fedcbdae98b6aeea37b6edebb0e485b298eca`) is run 42.
At 50 ms per move over 1024 games it scored 71.8%, +163 Elo [146, 180], against
the same baseline (run 26 scored +143 [127, 160], run 38 +160 [145, 176] under
the same clock).

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
  number of pieces on the board (`(pieces - 1) / 4`); network files are format
  version 2, and a version-1 network converts exactly by replicating its
  single output layer.
- Iterative deepening stops on a mate score only once the depth covers the
  mate distance. A shallow iteration could inherit a longer mate from the
  table, and replaying it shuffled won queen endings into a threefold
  repetition; the default network now mates KQ v K from `4k3/8/8/8/8/8/3Q4/4K3`
  in 15 moves at 50 ms per move where it previously drew.
- Accumulator adds, removes and the clipped dot product dispatch to AVX2 at
  runtime when the CPU supports it; the arithmetic is unchanged.

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
records the corpus (192 seed groups of 4096 games: 64 taught by the handcrafted
engine at 50k nodes, then 32, 16, 16 and 16 taught at 50k nodes and 16, 16 and
16 at 100k nodes by successively stronger published networks) and the training
settings (40 epochs, batch 8192, rate 0.0057 decaying by 0.9 per epoch, λ 0,
K 0.88). A whole run (prepare, train, match, style, throughput) takes about ten
minutes on 96 cores; the corpus members are cached per seed group.

## Style

The fixed-node fixtures in `tests/data` pinned handcrafted-era outputs and were
re-pinned to the network; each published network moves the 20,000-node
knife-edge positions again, so the profile-control fixtures are mined from
self-play against the shipped network: positions where Aggression 100 invests
material and Aggression 0 does not (`knight-f6-investment`,
`rook-c3-investment`), declines an unsound king-side sacrifice with the same
move as Aggression 0 (`unsupported-bishop-f6`, `unsupported-bishop-b7`), keeps
the queens on where Aggression 0 trades them
(`avoid-queen-trade-for-knight-d5`), and pushes a pawn where Aggression 0
moves a piece (`central-pawn-thrust`, `queenside-pawn-thrust`). The
standard-profile acceptance suite has two sacrifices that Aggression 75 makes
within its ceiling (`standard-bishop-f2-investment`,
`standard-queen-d5-investment`). The
verified-null contract allows a 25-centipawn score drift with an unchanged
best move, and its in-check position has a single winning capture.

The 75-over-0 forcing-move ratio of the shipped network is 1.08 over 512 games
(handcrafted 1.06); its forcing-move rate against the handcrafted engine is
1.15 times the handcrafted engine's own.

## Limitations

- Every strength figure is against one frozen handcrafted binary at short
  controls; no absolute rating or long-time-control result is claimed.
- The corpus lives under `artifacts/` and is regenerated deterministically by
  the recipe rather than stored in the repository; the teacher networks it
  depends on are kept under `artifacts/autoresearch/teachers`.
- The 256-unit network was not adopted because its fixed-node gain was within
  noise and it costs about 20% of throughput; it may pay off with more data.
- Data returns are small now: 22.9M to 34.2M rows moved the fixed-node result
  from +123 to +147, about +5 per 3M rows, all from groups labelled at 100k
  nodes. The remaining levers are deeper teacher labels and, under a clock,
  throughput.
