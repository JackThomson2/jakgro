# Measured search-efficiency series

## Verdict

This series makes the engine **search the same tree substantially faster and a
slightly smaller tree**, measured on the repository's own frozen efficiency
suite. It makes **no Elo claim**. No paired match was run, because
`cutechess-cli` is not installed on the measurement host and its network is
closed, so the strength question this series exists to serve remains open.

The two changes that dominate the result were verified to leave the searched
tree bit-identical, which is a stronger guarantee than a match: node counts,
selected moves, scores, and principal variations were unchanged in every
observation, so their entire effect is throughput.

## Provenance

| Input | Value |
| --- | --- |
| Series head | `dd57a81099d30ddf300ff8586cb34765cc5bc660` |
| Series base | `0bc2b80c72cd0c9a97f053e05f5ca583dc186945` |
| Head binary SHA-256 | `8b9586c678deff6e848d57ba7269913f7ce9c98939e66d527f1566b44d2aa067` |
| `cozy-chess` dependency | `7e93cdea094a50c1574081ceb6e7b269ad0234ee` |
| Build profile | `release` |
| Toolchain | `rustc 1.88.0 (6b00bc388 2025-06-23)` |
| Host | 96-core x86_64 Linux |

## Where the time was going

Profiling a default-profile search with `perf` over an eight-second middlegame
search attributed time as follows. This measurement, not intuition, chose the
first two patches:

| Symbol | Share |
| --- | ---: |
| `evaluation::features::extract` | 50.5% |
| `MoveFacts::search_metadata` and `MoveFacts::metadata` | 10.1% |
| `Board::play_unchecked_with_piece` | 5.0% |
| `see::best_capture_gain` | 2.6% |

Half of all search time was feature extraction, and roughly half of *that* was
computing attacking-style terms that search then multiplied by zero: search
always scores through `EvaluationConfig::objective_scoring`, which forces
aggression to zero.

## Per-patch results

Each row is the repository efficiency suite at depth five, five samples, 500 ms
fixed-time probe, measured against the immediately preceding commit.

| Patch | Profile | Node reduction | NPS change | Depth change |
| --- | ---: | ---: | ---: | ---: |
| release profile | 75 | 0.000% | +4.5% (fixed-depth wall time) | 0.000 |
| style-free objective eval | 75 | 0.000% | +46.1% (fixed-depth wall time) | 0.000 |
| clock-bucketed table key | 75 | 2.074% | +1.125% | +0.500 |
| clock-bucketed table key | 0 | 5.947% | +0.284% | +0.200 |
| cached static evaluation | 75 | 0.759% | — | +0.100 |
| cached static evaluation | 0 | 0.779% | — | +0.200 |
| deferred check detection | 75 | 0.000% | +3.399% | 0.000 |
| deferred check detection | 0 | 0.000% | +3.346% | 0.000 |

The first two patches are reported as fixed-depth wall-time ratios over six
positions rather than as suite percentages, because both leave the tree
identical and the suite's node and depth channels are therefore exactly zero by
construction. Their fixed-depth speedups were 1.045x and 1.461x at the default
profile; the evaluation patch also measured 1.648x at Aggression 0 and 1.444x at
Aggression 100, with identical nodes, moves, scores, and principal variations in
all eighteen observations.

## Cumulative result

Series head against series base, same suite and settings:

| Profile | Node reduction | NPS gain | Mean depth gain |
| ---: | ---: | ---: | ---: |
| 75 | 2.818% | **+45.057%** | **+1.300 ply** |

Fixed-depth wall time over six positions fell from 16.0 s to roughly 10.5 s at
the default profile. A sixteen-position tactical suite drawn from Win at Chess
and Bratko-Kopec solved 9 of 16 at both the base and the head at one second per
move, so the added depth did not cost tactical sight on that sample. Nine of
sixteen is a weak absolute result and is reported as a control against
regression, not as a strength measurement.

## Rejected avenues

Two candidates were implemented, measured, and discarded. Both are recorded
because the measurement is the useful artifact.

**Asymmetric aspiration windows.** The re-search loop widens both bounds by
doubling around the previous iteration's score. Widening only the failing side
and recentring on the score the failed search returned is the conventional fix,
and per-iteration node ratios showed apparent spikes entering new depths that
looked like repeated root searches. Implemented and measured, it was neutral to
negative: 0.099% node reduction and −1.417% NPS at Aggression 75, and
**−0.100 ply** at Aggression 0. A per-iteration trace showed one test position
byte-identical, because aspiration never failed there at all, and the other
within noise but slower in wall time. The apparent spikes were ordinary
iterative-deepening growth. Existing telemetry had already reported
`aspiration_research_nodes` at 0.0–1.7% of all nodes, which bounded the
available gain before the patch was written; that measurement should have been
believed first.

**A make/unmake board layer.** This is the standing first item on the roadmap's
search-efficiency list. The `cozy-chess` fork does expose `Board::save_state`
and `Board::restore_state`, so it is implementable without patching the
dependency. It was rejected on measurement: `size_of::<Board>()` and
`size_of::<BoardState>()` are both 104 bytes, so a snapshot is exactly as large
as the board it would avoid copying, and `play_unchecked_with_piece` accounts
for 5.0% of profile. The ceiling is low and the cost is threading `&mut Board`
through `negamax`, `quiescence`, `see.rs`, and `tactics.rs`. Deferred in favour
of the two changes that addressed 60% of profile instead.

## Deferred: reduction scaling and late-move pruning

Replacing the fixed-threshold late-move-reduction ladder with a reduction scaled
by depth and move index, plus a movecount-based quiet skip, is the largest
remaining tree-shape gain and is **deliberately not included**.

Screened against the series head over five parameter variants, the best measured
**38.6% fewer nodes and +1.000 ply at both profiles**, with +1.50 ply mean at a
two-second fixed time and no tactical solves lost on the sixteen-position suite.

It is withheld because the movecount-pruning half damages the attacking
personality, which is the engine's stated purpose:

- Aggression 100 stops playing the `c4f7` bishop sacrifice in
  `r1bq1rk1/ppp2ppp/2n2n2/2b1p1NQ/2B1P3/2NP4/PPP2PPP/R4RK1 w - - 0 10`,
  choosing `g5f7` instead. That position is the only required positive
  sacrifice in the acceptance contract, whose
  `sacrifice_required_positive_hits` is 1.
- Five of sixteen `personality.epd` fixtures stop matching, including
  `black-king-pressure`, where the Aggression 0 and Aggression 100 choices
  collapse onto the same move.
- `tools/measure_acceptance.py --check` fails on it and passes at both the
  series base and the series head.

The mechanism is not a stale expectation. `styled_root_node_limit` derives the
root personality budget from `objective_nodes / 5`, so cutting objective nodes
by 38% proportionally starves the sacrifice probing and verification that
produce the attacking style. Any future attempt must decouple that budget from
the objective node count, or exempt styled root candidates from the new pruning,
before that depth gain can be accepted.

### The reduction half alone was screened separately

Isolating the scaled reduction, with the movecount pruning removed, **attributes
the whole personality regression to the pruning half**. Reduction-only keeps all
sixteen `personality.epd` fixtures, keeps the required `c4f7` sacrifice at
Aggression 100, and passes both the style and acceptance gates with zero failing
positions. It gained +0.500 ply mean at a two-second fixed time.

It is still not included, because its node effect is a per-position gamble rather
than a uniform gain, and the repository's depth-five efficiency suite cannot see
it at all:

| Position | Depth-5 nodes | Depth-9 nodes |
| --- | ---: | ---: |
| start position | 0.0% | −42.8% |
| Kiwipete | 0.0% | **+17.0%** |
| middlegame | 0.0% | −58.7% |
| closed centre | 0.0% | −33.4% |
| total | 0.0% | −6.3% |

The scaled formula coincides with the old ladder at shallow depths, so the
frozen suite at depth five reports exactly 0.000% node reduction and cannot
discriminate this change; the effect appears only deeper. There it helps three of
four positions substantially and hurts the fourth by 17%, which is a variance
profile that needs a paired match to judge rather than a node count. That match
is exactly what this host cannot run, so the change is left for a host that can.

Several other fixtures did move for a defensible reason, and are recorded here
so a future attempt does not mistake them for regressions. `punish-central-queen`
still finds `d1d5` but scores it 1290 rather than 1274, converging toward 1312
against the base's 1301 at depth twelve while using 3.2x fewer nodes, so the new
number is the better estimate. `starting-development` and `central-space` pinned
shallow-depth artifacts: the base engine itself abandons `e2e3` for `d2d4` by
depth eight, and both engines abandon `f1c4` for `b1c3` by depth ten.

## A note on the efficiency suite's depth

`tools/measure_search_efficiency.py` defaults to depth 4 and was run here at
depth 5. That is adequate for the patches in this series, whose effects are
either tree-identical or present at every depth, but the deferred reduction
change above is invisible to it: the scaled formula coincides with the old ladder
at shallow depths, so the suite reported exactly 0.000% while a depth-9 probe
showed swings from −58.7% to +17.0%. A selectivity change should be screened at a
depth where it actually differs from its parent before the suite's verdict is
taken as evidence either way.

## What would settle the strength question

Nothing in this document is an Elo claim, and the node and depth gains here must
not be read as one. Establishing strength requires paired fixed-time matches on
a host with `cutechess-cli`, through the existing workflow:
```sh
python3 tools/run_match.py \
  --engine /path/to/head/jakgro \
  --baseline-engine /path/to/base/jakgro \
  --candidate-aggression 75 \
  --baseline-aggression 75 \
  --games 96 \
  --time-control 0.25+0.002 \
  --hash 16 \
  --pgn match.pgn \
  --manifest match.manifest.json

python3 tools/analyze_match.py \
  --pgn match.pgn \
  --manifest match.manifest.json \
  --json match.summary.json
```

Because this series is predominantly a throughput improvement, a fixed-node
match cannot detect it: at equal nodes the head and base search identical trees
for the two largest patches. The match must be run at a fixed time control.

## Validation

The series head passed:

```sh
cargo fmt --check
cargo clippy --all-targets --locked
cargo nextest run
cargo test --doc
python3 -m pytest
python3 tools/validate_acceptance_contract.py
python3 tools/measure_style.py --engine target/release/jakgro --check
python3 tools/measure_acceptance.py --engine target/release/jakgro --check
```

All 192 Rust tests pass, the style and acceptance gates pass with no failing
positions, and the pinned suite hashes are unchanged, because this series
re-baselines no fixture.
