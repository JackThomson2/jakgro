# Third measured strength series

## Verdict

One change has landed so far. Against the second series' head over 4096
colour-reversed games at a fixed 50 ms per move, the result is **+22.7 Elo
[15.6, 29.8] at Aggression 75** and **+28.1 Elo [21.2, 34.9] at Aggression 0**.
Both cross the predeclared `elo0=0, elo1=20, alpha=beta=0.05` H1 boundary, both
report a likelihood of superiority of 100%, and neither match recorded a fault.

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

The honest predictor in this series is **completed depth at a fixed time**, the
`depth-gain` channel `measure_search_efficiency.py` already reports. The one
patch that landed gained +0.200 ply and +22.7 Elo. Both patches that were
rejected gained +0.100 ply or less while cutting nodes substantially, and that
channel — not the node count — is what identified them.

## Provenance

| Input | Value |
| --- | --- |
| Series base | `f920768` |
| Base binary SHA-256 | `e6f58b684ed0dfc2…` |
| Candidate revision | `9a99848` |
| Candidate binary SHA-256 | `17be6eb6bafd2f0a…` |
| Dependency revision | `7e93cdea094a50c1574081ceb6e7b269ad0234ee` |
| Toolchain | `rustc 1.96.0 (ac68faa20 2026-05-25)` |
| Host | 10-core arm64 macOS, concurrency 8 |
| Opening corpus | `data/selective-search-confirmation.epd`, 2048 positions |
| Artifacts | `data/series-three-quiescence-see-a75.sprt.json`, `data/series-three-quiescence-see-a0.sprt.json` |

## What landed

| Patch | Aggression 75 | Aggression 0 | Mechanism |
| --- | --- | --- | --- |
| Quiescence exchange pruning | +22.7 [15.6, 29.8] | +28.1 [21.2, 34.9] | 48.5% fewer nodes, +0.200 ply |

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

`open-game-style` pins the score of whatever iteration completes inside 20,000
nodes. The same move and the same principal-variation prefix now come from depth
five rather than depth four, so the pinned score moved with the depth. That
fixture is a direct record of the patch working.

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
- The corrected calibration is an observation from two rejections and one
  acceptance, not a fitted slope. It says the old slope over-predicts here; it
  does not yet say by how much in general.

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
