# Fifth measured strength series

## Verdict

Two changes landed out of six measured, and one of them is the first
search patch since the second series to survive a match. Measured head
against base over 4096 colour-reversed games at a fixed 50 ms per move, the
series is worth **+39.3 Elo [32.5, 46.1]** at Aggression 75, LLR 49.9, and
**+33.5 Elo [27.0, 40.1]** at Aggression 0, LLR 42.7, both accepting H1 with
no faults. A clocked channel at `1.0+0.01` over 2000 games measures
**+37.3 Elo [27.1, 47.6]**, LLR 20.4, accept H1.

The engine did not become duller for it. Over the Aggression 75 match it
plays 31.12 forcing moves per hundred against the base's 30.25, and 10.80
checks against 10.17: 103% forcing retention and 6% more checks, forty Elo
stronger. The measured cost of the default profile against objective play is
-67.7 Elo [-77.4, -58.1] on the head against -68.1 Elo [-77.7, -58.6] on the base, over 2000 games each.

Per patch, each against its immediate parent:

| Patch | Aggression 75 | Aggression 0 |
| --- | --- | --- |
| Eight evaluation features, tree-identical | not measured alone | not measured alone |
| Evaluation refit, fourth series' corpus | **+18.8 [12.3, 25.3]** | **+19.8 [13.2, 26.4]** |
| Move ordering carried across a game | **+10.4 [3.9, 16.8]** | not measured |
| Null probe shortened by the margin above beta | -1.9 [-8.2, +4.5] | not measured |
| Aspiration window narrowed, re-centred on the fail | screened out | screened out |
| Reductions deeper at cut nodes and where nothing improved | screened out | screened out |
| Follow-up history keyed by the side's own move | screened out | screened out |

The fourth series ended by saying the next headroom was again in the
evaluation, and named safe-square mobility, an objective storm and king
tropism. All three are in this series, with five more blocks beside them,
and the refit that gives them values is worth twenty Elo on the corpus the
fourth series already fitted. That is what more knowledge of the same games
buys; the corpus from stronger play is the next series' first commit. The
search patch that landed is the one the frozen suite could not see, and the
one the suite liked best lost its match; the third series' rule that the
cheap channels screen and do not decide now has its converse on the record.

## Provenance

| Input | Value |
| --- | --- |
| Series base | `f3166f6` |
| Base binary SHA-256 | `4132e352ef56ae61…` |
| Series head | `70acf30`, engine sources as at `a244947` |
| Head binary SHA-256 | `25d950b3cd6503af…` |
| Dependency revision | `7e93cdea094a50c1574081ceb6e7b269ad0234ee` |
| Toolchain | `rustc 1.96.0` |
| Host | 10-core arm64 macOS, concurrency 8 |
| Opening corpus | `data/selective-search-confirmation.epd`, 2048 positions |
| Tuning corpus | 1,840,913 positions, the fourth series' corpus read into the new vector unchanged |
| Artifacts | `data/series-five-refit-a75.sprt.json`, `data/series-five-refit-a0.sprt.json`, `data/series-five-carry-history-a75.sprt.json`, `data/series-five-null-margin-a75.sprt.json`, `data/series-five-cumulative-a75.sprt.json`, `data/series-five-cumulative-a0.sprt.json`, `data/series-five-cumulative-clocked.sprt.json`, `data/series-five-personality-head.sprt.json`, `data/series-five-personality-base.sprt.json`, `data/series-five-gate.json` |

## What landed

### The evaluation knowledge

Eight commits added sixty-seven parameters to the objective evaluation at
values that reproduced the existing score exactly, each gated tree-identical
against the series base at depth eight over the frozen suite at both
profiles:

- the pawn storm against the king graded by the nearest enemy pawn's rank
  distance on the king's file and on the adjacent files, with the storms a
  friendly pawn blocks counted apart;
- safe centre squares behind the pawn chain, alone and scaled by the
  owner's pieces;
- candidate passers by rank, pawn islands, and a king on a flank with no
  pawn of either colour;
- knights, bishops and rooks scaled by the pawn count, and bishops by the
  friendly pawns on their colour;
- moves onto pawn-attacked squares per piece type, counted beside the raw
  mobility the curves and the profile adjustment keep reading;
- passers by the safety and the freedom of their path, and a rook behind;
- pawn-push threats and castling rights;
- king tropism by piece and bucketed distance.

The fitter's vector grew from 544 to 611. Scalars added after the tables
append as one-length blocks rather than in the trailing scalar list, whose
length every later offset builds on, and each block added a position to the
round-trip suite in which it is non-zero, since two zero-valued blocks
swapped in the layout would otherwise pass. Measured together at a second a
position, seven samples a position, the eight blocks cost 2.1% throughput
at Aggression 75 and 3.3% at 0; the cache-resident ones cost nothing that
could be measured. Readings at 250 ms and three samples varied by three
percent either way on identical code and were not used to decide anything.

### The refit

Fitted on the fourth series' 1.84 million positions with K = 0.8636 against
the outcome alone, 45 of 611 features held at their published values for
fewer than 2,000 observations, anchored to a middlegame pawn of 94. Held-out
loss 0.077635, against 0.078270 for the old vector on the same positions.
The fit was written back with the new `tools/splice_weights.py`, which
replaces the hand paste that made the fourth series' fit two-stage by
accident, so this fit is a single stage on purpose.

The fit priced most of the new terms as a player would: a minor's move onto
a pawn-guarded square six to eight centipawns below an open one, a bishop
five per own pawn on its colour, the right to castle eighteen, a threatened
pawn push ten, a rook behind a passer five, a king on a pawnless flank
seventeen in the ending. The storm blocks came out small and positive for
the stormed side, which is recorded as the fit's finding rather than
corrected by hand. A `--lambda 0.5` alternate on the same corpus was built
beside it and not matched.

### The carried ordering

Every search built its move ordering from nothing. The main searcher's
histories now outlive the search, in a memory the engine shares as it shares
the table, and the next search takes them, halved and without the killers,
only if its root follows the remembered root in the same game, which the
position's hash history says. The same position searched again, or an
unrelated one, starts cold, so every fixed-node fixture and every tool that
walks a suite through one process measures what it measured. On the frozen
suite this is invisible by construction; a 400-game replay through the
arbiter completed 6.561 plies a move against 6.559, which is nothing, and
the match measured **+10.4 Elo [3.9, 16.8]**, LLR 4.9, accept H1.

## What did not land, and why

**The null probe shortened by the margin above beta.** One ply per two
hundred centipawns of static evaluation above beta, to at most three, on the
probe alone, with the verification's depth untouched. On the frozen suite it
saved 2.7% of nodes and half a ply at Aggression 75, the largest depth gain
any search patch has shown on that channel, and every gate passed. It
measured **-1.9 Elo [-8.2, +4.5]**, accept H0.

**The aspiration window.** A first radius of thirty rather than fifty,
re-centred on the failing score. Thirty and forty both lost three tenths of
a ply at the default profile, the fitted evaluation's scores moving more
between iterations than a narrower window allows, and following the fail
alone was inert. Screened, not matched.

**Reductions deeper at cut nodes and where nothing improved.** No form
gained depth at the default profile; the cut-node ply alone searched six
percent more nodes, the extra reductions buying re-searches. Screened, not
matched.

**A follow-up history keyed by the side's own move.** Under the agreement
rule the continuation history uses, it lost three tenths of a ply at the
default profile. Screened, not matched.

The screens are recorded with their numbers in
`data/series-five-screens.md`, and the four rejected patches are kept under
`data/series-five-patches/`.

## The gate

`tools/gate_strength_personality.py` binds the four channels — objective
and same-profile head against base, and each binary's default profile
against its own objective play — to the style comparison, the acceptance
suite and the efficiency summary, and passes: forcing retention 102.9%
against the 90% floor, every safety and anti-sacrifice control preserved,
the candidate's personality cost -67.7 against the base's -68.1 (delta +0.4
against the -35 allowed), and the acceptance root loss 22 inside its cap of
45. The verdict is `data/series-five-gate.json`.

The efficiency channel, head against base over the frozen suite at depth
eight and 500 ms, reads node reduction 2.0% at Aggression 75, completed
depth +0.1 ply, and throughput -7.5%. The trees differ, so the saving is not
a speedup, and the throughput cost is the positions the refit visits rather
than the blocks' arithmetic, which measured -2.1% tree-identical; the
matches above are net of it.

## Fixtures that moved

The refit moved six search-regression records and eight personality
records, re-pinned in one commit with the reason for each, probed at five
and twenty times the budget against the base. Five search records keep
their move and take the new evaluation's score; `punish-central-queen`
captures the queen on d5 with the queen rather than the knight, which the
base itself does at five times the budget. In the 1.e4 e5 family
`starting-development`, `early-king-pressure`, `black-king-pressure` and
`kingside-pawn-storm` have no profile discrimination at their budgets;
`open-king-gambit` regains it at a hundred thousand nodes and its budget was
raised there, as the third series raised `central-space`, keeping five of
sixteen records discriminating against the suite's 30% floor.
`unsupported-greek-gift` plays b1d2 at both profiles at every budget probed
where the base plays c1f4 at every one, a conviction of the new evaluation
and neither move the sacrifice; the control holds with the profiles agreeing
and zero loss, and the style comparison now judges a control the suite was
re-pinned away from by the suite rather than by the baseline's move.

## Limitations

- Measured at 50 ms per move on a 10-core host at concurrency 8, with the
  clocked confirmation at `1.0+0.01`. Nothing reaches tournament controls.
- The refit is on the fourth series' corpus, the engine's own play at 50 ms.
  A corpus from the head at 200 ms is the obvious next step and the fitter
  reads it unchanged; it was not generated in time for this series.
- The carried ordering was matched at Aggression 75 only.
- Four of five search patches lost depth at the default profile on the
  frozen suite, and the one that gained it lost its match. The tree remains
  at what the evaluation supports.

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
  --pgn artifacts/series-five-a75.pgn \
  --summary-json artifacts/series-five-a75.sprt.json
```

Swap both aggression flags to `0` for the objective channel, or replace
`--movetime-ms 50` with `--time-control 1.0+0.01` for the clocked
confirmation. The refit:

```sh
cargo build --release --locked --features tuning --bin tune
./target/release/tune fit --positions artifacts/tuning/series-four-combined.txt \
  --out artifacts/tuning/series-five-fit.txt --epochs 1200 --l2 1e-7
python3 tools/splice_weights.py artifacts/tuning/series-five-fit.txt
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
