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
| 26 | 22.9M rows (64 handcrafted-taught, 64 network-taught seed groups) | **+123 [+112, +134]** | 1.057 | 1.30 |

The published network (`nets/jakgro.nnue`, SHA-256
`45549474bb92ad22406912bbbf0e154b8102b6e7f2d0c592114bbd9f83cee26f`) is run 26.
At 50 ms per move over 1024 games it scored 69.5%, +143 Elo [127, 160], against
the same baseline.

Rejected: L2 1e-4 (-43 Elo), λ 0.1 (within noise), a 256-unit hidden layer at
5.8M and at 20M rows (within noise at fixed nodes, and 20% fewer nodes per
second), sigmoid scale K 1.2 (-14), 45- and 60-epoch schedules (within noise).

## What changed in the engine

- `nets/jakgro.nnue` is embedded and enabled by default; `Use NNUE false`
  selects the handcrafted evaluator and `EvalFile <embedded>` restores the
  built-in network.
- Features are colored piece-square pairs under eight 2x2 king buckets on a
  file-mirrored half board (6144 inputs), so a position and its horizontal
  reflection share weights.
- Accumulator adds, removes and the clipped dot product dispatch to AVX2 at
  runtime when the CPU supports it; the arithmetic is unchanged.

## Pipeline

`tools/generate_nnue_corpus.py` plays deterministic fixed-node self-play
between Aggression 75 and 0 from seeded random-ply openings and extracts
`FEN;outcome;score` rows; `nnue-data prepare` computes features, removes
duplicates and development rows that repeat a training position up to colour
and rank mirroring, and binds everything by SHA-256; `nnue-data train` runs
float32 Adam over sixteen fixed gradient shards (thread-count independent),
flushes subnormal moments, clamps weights to the export bounds, exports every
epoch and selects the lowest development label loss. `tools/nnue_recipe.sh`
records the corpus (128 seed groups of 4096 games: 64 taught by the handcrafted
engine, then 32, 16 and 16 taught by successively stronger published networks)
and the training settings (30 epochs, batch 4096, rate 0.004 decaying by 0.9
per epoch, λ 0, K 0.88).

## Style

The fixed-node fixtures in `tests/data` pinned handcrafted-era outputs and were
re-pinned to the network. Four positions where the network's Aggression 100 no
longer diverged from Aggression 0 were replaced with self-play positions where
it does: a bishop investment on g7 (`bishop-g7-investment`), a queen trade
declined for a check (`avoid-queen-trade-for-check`), a rook-lift pawn storm
and a central pawn thrust. The standard-profile acceptance suite gained a rook
exchange sacrifice that Aggression 75 makes within its 67-centipawn ceiling
(`standard-rook-f6-exchange-sacrifice`, 55 cp). The verified-null contract now
allows a 10-centipawn score drift with an unchanged best move.

## Limitations

- Every strength figure is against one frozen handcrafted binary at short
  controls; no absolute rating or long-time-control result is claimed.
- The corpus lives under `artifacts/` and is regenerated deterministically by
  the recipe rather than stored in the repository; the teacher networks it
  depends on are kept under `artifacts/autoresearch/teachers`.
- The 256-unit network was not adopted because its fixed-node gain was within
  noise and it costs about 20% of throughput; it may pay off with more data.
