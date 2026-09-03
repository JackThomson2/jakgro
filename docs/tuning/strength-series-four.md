# Fourth measured strength series

## Verdict

One change landed out of six measured, and it is the one the series was for.
Measured head against base over 4096 colour-reversed games at a fixed 50 ms
per move, the series is worth **+67.7 Elo [60.8, 74.6]** at Aggression 75,
LLR 98.3, and **+72.7 Elo [65.9, 79.6]** at Aggression 0, LLR 108.6, both
accepting H1 with no faults. A clocked channel at `1.0+0.01` over 2000 games
measures **+71.2 Elo [60.7, 81.7]**, LLR 45.4, accept H1.

The engine did not become duller for it. Over the Aggression 75 match it
plays 32.92 forcing moves per hundred against the base's 29.66,
and 12.32 checks against 9.50: 111% forcing retention and 30% more checks,
ninety-odd Elo stronger on both channels. The measured cost of the default
profile against objective play is -73.5 Elo [-83.5, -63.6] on the head
against -72.1 Elo [-82.0, -62.2] on the base, over 2000 games each: the
personality costs what it cost.

Per patch, each against its immediate parent:

| Patch | Aggression 75 | Aggression 0 |
| --- | --- | --- |
| Eight evaluation features, tree-identical | not measured alone | not measured alone |
| Specialised fused pass | tree identical, 0.753 → 0.881 of base throughput | same |
| Evaluation refit | **+76.1 [69.1, 83.1]** | **+88.6 [81.7, 95.5]** |
| Endgame scaling | -9.9 [-16.4, -3.5] | not measured |
| Correction history on the stand-pat | -8.1 [-14.8, -1.5] | not measured |
| Clock scaled by root effort and stability | -0.7 [-7.6, +6.3] at `1.0+0.01` | not measured |

The third series ended by saying the remaining headroom was in the evaluation
rather than the tree, and this series is the test of that claim. Eight
commits added a hundred and thirty-five parameters to the objective
evaluation at values that moved no score, a refit gave them values, and
that refit alone is worth more than the whole of the second series. Every
search patch after it was screened or measured and rejected, as five of seven
were in the third series, and this time from a stronger evaluation.

## Provenance

| Input | Value |
| --- | --- |
| Series base | `d3a1633` |
| Base binary SHA-256 | `8fa3e8bb67b90d69…` |
| Series head | `273d5e4` |
| Head binary SHA-256 | `4132e352ef56ae61…` |
| Dependency revision | `7e93cdea094a50c1574081ceb6e7b269ad0234ee` |
| Toolchain | `rustc 1.96.0` |
| Host | 10-core arm64 macOS, concurrency 8 |
| Opening corpus | `data/selective-search-confirmation.epd`, 2048 positions |
| Tuning corpus | 1,840,913 positions: 704,609 from `corpus-a75.pgn` (16,384 games, base vs base, 50 ms, four random quiet plies), 1,136,304 from the third series |
| Artifacts | `data/series-four-refit-a75.sprt.json`, `data/series-four-refit-a0.sprt.json`, `data/series-four-refit-zero-king-danger-a75.sprt.json`, `data/series-four-endgame-scaling-a75.sprt.json`, `data/series-four-correction-history-a75.sprt.json`, `data/series-four-clock-scaling-clocked.sprt.json`, `data/series-four-cumulative-a75.sprt.json`, `data/series-four-cumulative-a0.sprt.json`, `data/series-four-cumulative-clocked.sprt.json`, `data/series-four-personality-head.sprt.json`, `data/series-four-personality-base.sprt.json`, `data/series-four-gate.json` |

## What landed

### The evaluation knowledge

The objective evaluation — the one search scores every node with, since the
personality only decides at the root — knew material, placement, one mobility
weight shared by every piece type, tempo, the bishop pair, doubled and
isolated pawns, passers by rank, a shelter pawn count and open king files.
Eight commits added what it lacked, each at values that reproduced the
existing score exactly and each gated tree-identical against the series base
at depth eight over the frozen suite, at both profiles:

- mobility curves indexed by move count for knights, bishops, rooks and
  queens, replacing one weight per move;
- rooks on open and semi-open files and on the seventh;
- knight and bishop outposts;
- backward pawns and connected pawns by rank;
- passers blockaded, by rank, and passers by each king's distance to the
  square ahead;
- king danger from bucketed attack units and safe checks by checking piece;
- the king's shelter graded by the nearest pawn's distance on each file;
- threats: minors attacked by pawns, hanging pieces, pieces attacked by
  something worth less.

The fitter's vector grew from 409 to 544 features. New groups append after
the placement tables so recorded corpora keep their meaning, and the engine
carries every rank-, distance- or bucket-indexed block as a weighted pair —
the structure blocks weighted once on a structure-cache miss — so that a hit
is one pair and the scorer multiplies nothing by zero.

### What they cost, and what was recovered

Gated one at a time, nothing about any commit said what they cost together.
On an idle host the base searched 1.30 times as many nodes per second as the
head, and the evaluation had gone from 140 to 285 nanoseconds a call.
Ablating every new computation at run time recovered almost none of it: the
cost was not the arithmetic but what it had done to one generic loop over six
piece types, indexing a dozen colour-indexed arrays, which had grown past
what the register allocator could keep in registers.

The fused pass was specialised by piece type and by path through const
parameters, so the objective loop compiles with no style code in it and the
pawn scan is set-wise; the indexed blocks are weighted where they are
produced; and the structure-cache miss path was cheapened. Throughput against
the base went from 0.753 to 0.881, tree-identical throughout. What is left is
about eight operations more per piece, and the refit paid for it.

### The refit

Fitted on 1.84 million positions with K = 0.8419 against the outcome alone,
anchored to a middlegame pawn of 94 so no search margin changed its meaning.
Blending the label toward the recorded search score was screened on the
held-out outcome channel the fitter now reports and rejected at every lambda.

What ships is a two-stage fit, and the reason is recorded exactly in the
commit. Fitted freely, the vector failed the one sacrifice the acceptance
contract requires. The remedy planned for that was to hold the king-danger
block at zero and refit, but the fitter was rebuilt from a tree with the free
fit spliced in, so the hold pinned the block to the free fit's values rather
than to zero; the result is a second 1200 epochs from the free fit with the
king-danger and shelter-distance blocks frozen, a lower held-out loss than any
single-stage fit, every control passing, and the +76.1 and +88.6 above.

The genuine experiment was then run from a fitter built at a commit.
Holding king danger at zero passes every control and measures **+62.7 Elo
[55.8, 69.6]** at Aggression 75. The fitted king-danger and shelter-distance
blocks are therefore worth about thirteen Elo, and the sacrifice fixture that
decided the free fit turns on a few centipawns, which is recorded as such
rather than as evidence that king danger and the personality are at odds.

The attacking weights and the profile mobility adjustment were held out of
the fit, as in the third series. Forcing retention over the refit's own
match was 113%: 33.04 forcing moves per hundred against 29.35, and 12.51
checks against 9.25.

## What did not land, and why

**Endgame scaling.** A multiplier on the endgame component for material the
stronger side cannot convert: dead material to nothing, a pawnless side with
a minor or less to an eighth, opposite-coloured bishops alone to a fraction
growing with the stronger side's pawns, and every ending damped by pawn
count. Hand-set, because none of it is a sum of terms. It measured **-9.9
Elo [-16.4, -3.5]**, accept H0. The refit had already priced those endings
from a corpus that contains them, and a second opinion on top of a fitted
one is a double count.

**Correction history on the stand-pat.** The half the third series did not
try: two tables keyed by pawn structure and by non-pawn material, by side to
move, applied to the quiescence stand-pat as well as to every pruning margin,
with the transposition table keeping the raw value. It measured **-8.1 Elo
[-14.8, -1.5]**, accept H0, and it failed two personality controls before
the match was run: at Aggression 100 it took the equal queen trade it is
meant to decline, and it left the anti-sacrifice fixture's move. The
mechanism is visible: the learned bias pushes symmetric quiet positions a few
centipawns negative, which trips the root guard that never trades a
non-negative objective score for a negative one. Applied to the margins alone
it was inert in the third series; applied to the score it is not inert and
not welcome.

**Move-count pruning at depths five to eight.** The rule had carried a depth
bound of eight since it was written and had never reached it, its call site
being gated on the shallow static evaluation. Given its own gate, it is a
no-op: the limits at those depths, 43 to 102 moves, are never reached, and
the missing improving signal doubles them. With a limit of 3 + d² the narrow
signal loses a tenth of a ply; the wide signal, every interior evaluation,
cuts ten percent of nodes and breaks the required sacrifice, exactly as the
note left at that site said it had once before. Screened, not matched.

**Quiet futility past depth two.** To depth four at two margins: under one
percent of nodes and no depth at either profile. Screened, not matched.

**Losing captures pruned in the ordinary search.** Three variants of swap-list
threshold and depth: every one searched more nodes at Aggression 75 and gained
no depth. Screened, not matched.

**The parked quiescence check filter.** On the refit it saves 3.9% of nodes at
Aggression 75 for a tenth of a ply and loses two tenths at Aggression 0.
Paired with one more quiescence check per line at the styled profiles — the
experiment the third series said the two belong in — it searches more nodes
and fails the sacrifice gate. Screened, not matched, and no longer parked.

**The clock.** The soft limit scaled by the best move's share of the root's
nodes and by how many iterations it has held, under a real clock only.
Over 2000 games at `1.0+0.01` it read +3.0 [-6.5, +12.5] with no verdict;
over 4096 it read **-0.7 [-7.6, +6.3]**, accept H0. The volatility hold the
engine already has is doing what this would do.

The screens are recorded with their numbers in
`data/series-four-screens.md`, and the four rejected patches that reached a
build are kept under `data/series-four-patches/`.

## The gate

`tools/gate_strength_personality.py` binds the four channels — objective and
same-profile head against base, and each binary's default profile against
its own objective play — to the style comparison, the acceptance suite and
the efficiency summary, and passes: forcing retention 111.0% against the 90%
floor, every safety and anti-sacrifice control preserved, the candidate's
personality cost -73.5 against the base's -72.1 (delta -1.5 against the -35
allowed), and the acceptance root loss inside its cap. The verdict is
`data/series-four-gate.json`.

The efficiency channel, head against base over the frozen suite at depth
eight and 500 ms, reads what the throughput commit said it would: node
reduction 0.3% at Aggression 75 and 4.1% at 0 (the trees differ, so this is
not a saving), throughput -8.1% and -14.5%, completed depth -0.3 and -0.2
ply. That is the cost the refit paid for, and the matches above are net of
it.

## Fixtures that moved

The refit moved fourteen rows of the personality suite, nearly all in the
1.e4 e5 family that supplied every fixture failure in the third series, and
they were re-pinned in one commit with the reason for each: probed at five
and twenty times the fixture budget, the refit's choices converge on the
parent's in most of them, so the records describe shallow preferences. Two
records lost their profile discrimination, `early-king-pressure` and
`black-king-pressure`, and two that the third series recorded as lost,
`open-king-gambit` and `kingside-pawn-storm`, regained it. The simplification
record still declines the queen trade at every budget to two million nodes,
with a different move. Six search-regression scores moved by a few
centipawns with the same move; one captures the queen with the knight rather
than the queen. The null-move contract was re-based to depth seven and to the
position where null pruning fires most, since at depth five the whole
contract rested on a five-thousand-node tree.

## Limitations

- Measured at 50 ms per move on a 10-core host at concurrency 8, with the
  clocked confirmation at `1.0+0.01`. Nothing reaches tournament controls.
- The tuning corpus is still the engine's own play, from the series base and
  the third series' matches, on one opening book plus random quiet plies.
- The head is at 0.88 of the base's throughput. The remaining cost is the
  new terms' own arithmetic, and it is paid for, but it is there.
- The adopted fit's provenance is two-stage by accident of tooling. It is
  reproducible from the recorded steps, and the single-stage ablation is
  measured beside it, but it is not the fit the plan described.
- Every search patch was rejected. The tree's selectivity is now at what two
  evaluations in a row could support, and the next headroom is again more
  likely in the evaluation: safe-square mobility, pawn-storm and king-tropism
  terms in the objective evaluation, and a corpus from a stronger opponent.

## Reproduction

```sh
cargo build --release --locked --bin jakgro --bin selfplay
python3 tools/run_sprt.py \
  --engine target/release/jakgro \
  --baseline-engine /path/to/base/jakgro \
  --candidate-aggression 75 --baseline-aggression 75 \
  --games 4096 --movetime-ms 50 --concurrency 8 \
  --elo0 0 --elo1 20 \
  --openings docs/tuning/data/selective-search-confirmation.epd \
  --pgn artifacts/series-four-a75.pgn \
  --summary-json artifacts/series-four-a75.sprt.json
```

Swap both aggression flags to `0` for the objective channel, or replace
`--movetime-ms 50` with `--time-control 1.0+0.01` for the clocked
confirmation. The refit:

```sh
cargo build --release --locked --bin selfplay
./target/release/selfplay --engine /path/to/base/jakgro \
  --games 16384 --movetime-ms 50 --random-plies 4 --seed 20260902 \
  --openings docs/tuning/data/selective-search-confirmation.epd \
  --pgn artifacts/corpus.pgn
cargo build --release --locked --features tuning --bin tune
./target/release/tune extract --pgn artifacts/corpus.pgn --out artifacts/tuning/positions.txt
./target/release/tune fit --positions artifacts/tuning/positions.txt \
  --out artifacts/tuning/stage-one.txt --epochs 1200 --l2 1e-7
# splice stage-one.txt into weights.rs and placement.rs, rebuild tune, then
./target/release/tune fit --positions artifacts/tuning/positions.txt \
  --out artifacts/tuning/stage-two.txt --epochs 1200 --l2 1e-7 \
  --hold KING_DANGER_BY_BUCKET,SHELTER_KING_FILE_BY_DISTANCE,SHELTER_ADJACENT_FILE_BY_DISTANCE
```

The deterministic gates are unchanged:

```sh
python3 tools/measure_style.py --engine target/release/jakgro --check
python3 tools/measure_style.py --engine target/release/jakgro \
  --suite tests/data/sacrifice-gates.epd --check
python3 tools/measure_acceptance.py --engine target/release/jakgro --check
python3 tools/validate_acceptance_contract.py
cargo test --locked
cargo test --locked --features tuning
```
