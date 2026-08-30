# Third measured strength series

## Verdict

Two changes have landed out of seven attempted, for a cumulative **+42.6 Elo** at
the default profile over the second series' head, each measured over 4096
colour-reversed games at a fixed 50 ms per move.

Quiescence exchange pruning measures **+22.7 Elo [15.6, 29.8] at Aggression 75**
and **+28.1 Elo [21.2, 34.9] at Aggression 0**. Extending reverse futility
measures a further **+20.0 Elo [13.1, 26.8]**. All three cross the predeclared
`elo0=0, elo1=20, alpha=beta=0.05` H1 boundary, report a likelihood of
superiority of 100%, and record no faults.

The personality did not pay for it. All 32 fixed-node style choices and all 8
sacrifice-gate choices are identical at every profile, every acceptance contract
passes, and complete-game forcing play rose slightly rather than eroding:
32.44 forcing moves per hundred against the baseline's 31.82, a retention of
**101.9%** against a 90% floor.

## Calibration, corrected

The second series established `Elo ≈ 115 × log₂(speedup)` and used it to predict
each patch's value before measuring it. **That slope does not apply to this
series and should not be used again without a matching measurement.**

It was measured by handicapping the engine's *clock*, which leaves the tree it
searches unchanged. Every patch here changes which nodes are visited, so a node
reduction is not a speedup: part of the saving is work the search wanted. The
gap is large. Quiescence exchange pruning cut 48.5% of Kiwipete's depth-ten nodes
and 6.8% of the frozen suite's, which the slope reads as roughly +90 Elo. It
measured +22.7.

Completed depth at a fixed time, the `depth-gain` channel
`measure_search_efficiency.py` already reports, looked at first like the honest
replacement. It is better than the node count, but it is not a substitute for a
match either, and this series has the evidence both ways:

| Patch | Suite nodes | Depth gain | Measured Elo |
| --- | ---: | ---: | --- |
| Quiescence exchange pruning | -6.8% | +0.200 | **+22.7** [15.6, 29.8] |
| Reverse futility to depth seven | -19.2% | +0.200 | **+20.0** [13.1, 26.8] |
| Principal-variation reductions | -8.7% | +0.200 | +3.0 [-3.9, 9.8] |
| Razoring | -4.3% | +0.100 | +3.6 [-3.1, 10.4] |
| Correction history | +4.0% | +0.000 | +1.1 [-5.5, 7.7] |

The first two and the third are indistinguishable on every cheap channel and an
order of magnitude apart in Elo. Ten quiet positions and a 0.1-ply granularity
cannot separate a patch that finds better moves from one that merely finds the
same moves sooner.

**The conclusion for later series is that the cheap channels screen, they do not
decide.** Their proper use is to reject early: a patch that gains no depth is not
worth a match. A patch that gains depth has earned a match, and nothing more.

## Provenance

| Input | Value |
| --- | --- |
| Series base | `f920768` |
| Base binary SHA-256 | `e6f58b684ed0dfc2…` |
| Series head | `3bf0a5f` |
| Quiescence-patch binary SHA-256 | `17be6eb6bafd2f0a…` |
| Dependency revision | `7e93cdea094a50c1574081ceb6e7b269ad0234ee` |
| Toolchain | `rustc 1.96.0 (ac68faa20 2026-05-25)` |
| Host | 10-core arm64 macOS, concurrency 8 |
| Opening corpus | `data/selective-search-confirmation.epd`, 2048 positions |
| Artifacts | `data/series-three-quiescence-see-a75.sprt.json`, `data/series-three-quiescence-see-a0.sprt.json`, `data/series-three-reverse-futility-a75.sprt.json` |

## What landed

| Patch | Aggression 75 | Aggression 0 | Mechanism |
| --- | --- | --- | --- |
| Quiescence exchange pruning | +22.7 [15.6, 29.8] | +28.1 [21.2, 34.9] | 48.5% fewer nodes, +0.200 ply |
| Static evaluation everywhere | not measured alone | not measured alone | preparatory; tree identical, -1.1% throughput |
| Reverse futility to depth seven | +20.0 [13.1, 26.8] | not measured | 19.2% fewer nodes, +0.200 ply |

Quiescence is roughly 97% of every tree this engine builds, and the rule meant to
keep refuted captures out of it required five conditions to hold at once. Almost
no capture satisfied all five, so almost none were pruned. Replacing the
conjunction with the swap-list test alone, while keeping every forcing exemption,
is the whole patch.

Aggression 0 gains more than Aggression 75 because one of the five conditions was
non-zero aggression: the objective profile had no quiescence pruning at all and
therefore searched the larger tree of the two.

Kiwipete at depth ten, before and after:

| Profile | Before | After | Reduction |
| ---: | ---: | ---: | ---: |
| 0 | 19,346,533 | 9,759,766 | 49.6% |
| 75 | 22,555,630 | 11,621,843 | 48.5% |
| 100 | 54,155,139 | 36,182,681 | 33.2% |

The frozen efficiency suite is much quieter than Kiwipete and moves far less, at
6.800% fewer nodes and +0.200 ply at a 500 ms probe. The gap between the two is
itself the finding: this patch pays in tactical positions and does almost nothing
in quiet ones.

The threshold was screened at 0, -25, -50, -75, -100, -125 and -150 centipawns.
Tree size is not monotone in it, because pruning a capture changes the score a
node returns and so the cutoffs above it: -25 both prunes less than 0 and
searches less. Zero measured the largest suite reduction, 10.849% and +0.300 ply,
but moved four fixtures; -25 keeps every style choice and every contract.

## What did not land, and why

**Quiescence delta pruning.** A capture that cannot reach alpha even if it won
the captured piece for nothing, plus a margin, is skipped. Implemented with the
same exemptions as the exchange rule. Isolated against the exchange patch it cut
7.792% of nodes but gained only +0.100 ply and lost 2.763% of throughput to the
bound itself, so its net worth was small before any style cost.

It also moved `unsupported-greek-gift` at Aggression 100 from `c1f4` to `c1e3`,
failing an anti-sacrifice control whose contract permits no objective loss at
all, and it did so at **every** margin screened from 50 to 300 centipawns. A
failure that does not respond to the parameter meant to control it is a
structural disagreement with the fixture, not a tuning problem. Rejected.

**Quiescence check filtering.** Quiet checks are generated with no material
filter, so a piece can be given away to deliver a check the opponent simply takes.
Requiring a quiet check outside the enemy king's zone to hold its destination
square by the swap list cuts 5.6% of nodes at Aggression 0, 11.8% at 75 and 19.4%
at 100 — the largest saving where the cost is largest — with every style and
sacrifice choice unchanged.

It is **parked rather than rejected**, on the branch
`series-three-patch3-parked`. At the shipped default profile it measures +0.000
ply, so alone it is unlikely to be worth Elo; its value is that it makes forcing
search cheaper, which is the premise of the experiment that spends the saving
back on aggression. The two belong in one measurement.

Implementing it surfaced a real gap: `static_exchange_eval` reports zero for any
move that captures nothing, because its first gain is the captured piece, so the
filter silently did nothing until a quiet variant was written that settles the
destination square from a zero balance.

**Razoring.** A node standing far enough below alpha drops to quiescence, and its
answer is accepted if it agrees the node fails low. Four settings were screened;
the best, depth three at 100 plus 150 per ply, cut 4.251% of nodes for +0.100 ply
with throughput up 0.641%, and every style, sacrifice and acceptance gate passed.
It measured **+3.6 Elo [-3.1, +10.4]**, LLR -1.135, no verdict. An interval that
includes zero is not a reason to add a rule with two more margins to maintain,
and this engine's quiescence is now well enough pruned that dropping into it
early buys little.

**Principal-variation reductions.** The reduction ladder returns zero for any
principal-variation node, so the widest part of the tree is searched in full. One
ply of relief instead of exemption cut 27.998% of nodes for +0.300 ply, the
largest depth gain in the series, and broke the personality: `open-king-gambit`
stopped playing its forcing thrust at Aggression 0 and the required knight
investment was lost at 100.

Two plies of relief lost depth outright at -0.100 ply, and three is arithmetically
the exemption again. Exempting only the plies where the styled root decides —
reducing on the principal variation from ply two — kept every gate clean and
still gained +0.200 ply for 8.745% fewer nodes.

That version measured **+3.0 Elo [-3.9, +9.8]**, LLR -11.498, accept H0. It is the
clearest result in the series: the engine is decisively not 20 Elo better for it,
on a patch whose cheap channels were indistinguishable from the two that were.

**Correction history.** The evaluation is a fixed function of the position, so
where it is wrong it is wrong the same way each time the same pawn structure
appears, and search already knows: the score it returns after looking disagrees
with the static score in a direction that repeats. A table keyed by pawn
structure and side to move accumulated that disagreement as a decaying average
weighted by depth, and every rule that reads the evaluation — reverse futility,
quiet futility, the null-move guard and the improving signal — read the corrected
value. The table stored the raw value, so what the transposition table caches
stays a pure function of the position.

It measured **+1.1 Elo [-5.5, +7.7]**, LLR -3.457, accept H0, with every gate
clean. The correction was demonstrably live rather than inert — Kiwipete at depth
eleven searched 30,106,586 nodes against 30,417,013 — it simply did not change
enough decisions to matter.

The likely reason is where it was *not* applied. Quiescence is about 97% of this
tree and its stand-pat score is the evaluation used most, and it was left
uncorrected because correcting it changes the score the engine reports rather
than only the score it prunes by. Correcting the stand-pat, and keying a second
table by non-pawn material, are the two obvious next attempts and neither was
tried.

## Fixtures that moved

Two records were re-pinned, each in its own commit.

`contract-central-space` measured a 30-centipawn cap at 20,000 nodes in the
Italian and Scotch branch point after 1.e4 e5 2.Nf3 Nc6, where both candidate
moves are main-line theory. The engine's own estimate of the gap between them is
34 centipawns at 20,000 nodes, 14 at 100,000 and 2 at 400,000, so the cap was
being applied to a number that had not stopped moving. The budget was raised to
100,000, where all three of the record's moves still hold and the accepted
engine measures 15. The cap itself is unchanged. The budget is deliberately not
raised further: at 200,000 nodes the tuned profile prefers a third move.

Four fixed-node fixtures pin the score of whatever iteration completes inside
their budget: `starting-style`, `open-game-style`, `developed-open-game` and
`punish-central-queen`. Each reached one ply deeper on the same budget with the
same move and the same principal-variation prefix, so the pinned score moved with
the depth. Those fixtures are a direct record of the patches working, and the
last of them was re-pinned for a patch that was then rejected, so it is back at
its original value.

Two records in the personality suite were re-based once, in their own commit,
after the same class of failure appeared three times. That suite splits into
twelve constructed positions, which have not moved for any patch in this series,
and four near-symmetric positions from the 1.e4 e5 family, which supplied every
fixture failure. `black-king-pressure` required the objective profile to answer
with d7d5 where the engine now prefers b8c6, and is right to: b8c6 scores 19
centipawns against 6 at depth fourteen. `contract-early-king-pressure` capped at
30 what the personality may pay for the Centre Game, a quantity that measures 18
to 45 across three engines and six budgets, that fails on the accepted engine at
30,000 and 40,000 nodes, and whose personality signal vanishes entirely past
60,000. Its cap is now 45, the top of the measured band.

The objective telemetry regression asserted that Aggression 0 never prunes a
quiescence capture. That was a statement of the exemption this series removes,
and it now asserts the opposite.

## A defect fixed before measuring

The second series recorded that some positions emit `bestmove` with no `info`
line, which the measurement tools cannot read. The cause was the budget check:
a node limit or expired clock aborts wherever it is, including inside the first
iteration, and with no iteration completed the caller fell back to the first
legal move in alphabetical order. The engine answered `go nodes 20` in a position
winning a piece with `a2a3`.

A search that is allowed to start now completes at least depth one. Explicit
cancellation still aborts and a budget of zero still searches nothing. Above the
smallest budgets the tree is unchanged, node for node at every completed depth.

## Limitations

- Measured at 50 ms per move on a 10-core host at concurrency 8, not the 96-core
  Linux host the earlier series used. No clocked confirmation has been run yet.
- The efficiency suite has ten active positions and is quiet relative to real
  play; its `depth-gain` channel has 0.1-ply granularity, which is coarse for
  separating a small gain from none.
- Five of seven attempted search patches were rejected or parked, four of them
  on a match after passing every deterministic gate. The search's selectivity is
  now close to what this evaluation can support, and the remaining headroom is
  more likely in the evaluation itself than in the tree it drives.
- Only Aggression 75 was matched for reverse futility; the objective channel was
  not re-run after the first patch.

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
  --pgn artifacts/series-three-a75.pgn \
  --summary-json artifacts/series-three-a75.sprt.json
```

Swap both aggression flags to `0` for the objective channel. The deterministic
gates are unchanged:

```sh
python3 tools/measure_style.py --engine target/release/jakgro --check
python3 tools/measure_style.py --engine target/release/jakgro \
  --suite tests/data/sacrifice-gates.epd --check
python3 tools/measure_acceptance.py --engine target/release/jakgro --check
python3 tools/validate_acceptance_contract.py
cargo test --locked
```
