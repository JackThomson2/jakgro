# Stored fail-low and razoring series

## Verdict

Two patches landed out of nine ideas measured in thirteen builds. Measured head
against the shipped base (`6e4c9a3`) over 8192 colour-reversed games at a fixed
50 ms per move, the series is worth **+10.1 Elo [+5.7, +14.5]** at Aggression
75, LLR 7.49, and over 4096 games **+18.5 Elo [+12.4, +24.6]** at Aggression 0,
LLR 8.32. Under the clock at `1.0+0.01`, 8192 games measure **+5.6 Elo [+1.2,
+9.9]**, LLR 3.12. All three accept H1 against (0, 5) with no faults. Both
patches were measured at 50 ms; the clocked channel is the only place either
was measured under a clock, and it credits them with about half of what the
fixed-time channel does.

The engine did not become duller for it. Over 4096 Aggression 75 versus 0
games at 50 ms the head plays 29.00 forcing moves per hundred against 25.69 for
its objective profile, a ratio of 1.129 against the base's 1.111 on the same
match, and 12.23 checks per hundred against 8.18. Against the base at
Aggression 75 it keeps 99.8% of the base's forcing moves and 99.4% of its
checks. The measured cost of the default profile against objective play is
-37.3 Elo [-43.6, -31.0] on the head against -38.5 Elo [-44.9, -32.1] on the
base.

## Why this series

The table, repetition and clock series closed on a warning: every change that
let the search trust the table more had lost about nineteen Elo, among them
storing the bounds of nodes that pruned or reduced moves without re-search.
This series took that result apart. The loss was not in the bounds but in how
the table took them. A fail-low names as its best move whichever move failed
least badly, and the store wrote that move over the one the table had
recorded. And any store of a position overwrote the entry it found unless that
entry was a deeper exact result, so a position reached again through a
reduction, or settled by quiescence at the horizon, replaced a deep bound and
its move with a depth-zero one. With a fail-low that names no move and a
same-position store that keeps a much deeper entry, storing every result is
worth six Elo.

The other half of the warning stands, and more firmly: reading a stored bound
in place of the static evaluation for pruning lost twenty to thirty Elo in all
three forms measured, with the fail-low bounds now in the table. The table is a
record of what was searched; it orders and cuts, but this engine's pruning
should not read it as an evaluation.

Razoring is the first new pruning rule to land since the search selectivity
series.

## Per patch

Each row is the patch against its immediate parent at Aggression 75 on both
sides, at a fixed 50 ms per move.

|Patch|Elo against parent|Games|
|---|---|---|
|Store fail-low results, keeping the recorded move|**+6.2 [+1.7, +10.6]**|8192|
|Razor hopeless shallow nodes into quiescence|**+6.6 [+2.3, +10.9]**|8192|

The storing patch's two halves were measured apart as well. The table half
alone — an inexact store at least four plies shallower than the entry it finds
for the same position keeps that entry, and a store without a move keeps the
recorded move — measured +1.1 [-5.0, +7.2] over the book and -1.5 [-7.2, +4.2]
over 4096 games with two random plies. The storing half on top of it measured
+5.6 [+1.3, +9.9] over 8192 games. They landed as one patch because the storing
half needs the table half, and the table half has no measured effect of its
own.

Razoring settles a non-PV node to depth three whose static evaluation lies more
than 200 + 150·depth² centipawns below alpha with a quiescence search on the
node's window, and returns that result only when quiescence confirms the
fail-low; a node quiescence lifts above alpha is searched in full. Over its
8192 games the default profile kept 99.4% of the parent's forcing moves (29.42
against 29.60 per hundred) and 97.7% of its checks. It crossed the upper bound
of the (0, 5) sequential test with an LLR of 4.24.

## Rejected directions

Each was measured against the head of the moment at Aggression 75, 4096 games
at 50 ms per move unless the row says otherwise.

|Candidate|Elo|
|---|---|
|Double extension of a clearly singular hash move, at most eight per line, and one ply less for a hash move that is not singular but whose stored score beats beta|-0.5 [-6.7, +5.7]|
|Best move of an iteration the clock abandons, when a later root move beat the first; 12288 games at `1.0+0.01`|+1.7 [-1.5, +4.9]|
|The same at 50 ms|+1.5 [-4.7, +7.7]|
|Pawn-structure correction history on the pruning evaluation, carried and aged with the move ordering|-1.2 [-7.3, +4.9]|
|SPSA over 24 search constants, 16,000 pairs at 50 ms; the tuned vector over 8192 games|-0.3 [-4.6, +4.0]|
|The same tuned vector at `1.0+0.01`|+4.6 [-1.7, +10.8]|
|Stored bound read in place of the static evaluation for pruning, exclusion searches included|**-30.2 [-36.6, -23.8]**|
|The same outside singular-extension exclusion searches|**-20.2 [-26.7, -13.8]**|
|The same, lower bounds and exact scores above the evaluation only|**-26.8 [-33.3, -20.3]**|
|Killers reduced one ply less than other quiets rather than exempt|-1.4 [-7.3, +4.5]|
|No null probe at a node the table bounds below beta, against the storing head, 8192 games|+2.6 [-1.5, +6.7]|
|The same against the razoring head, 12288 games, LLR -3.71|+0.3 [-3.2, +3.7]|

The interrupted-iteration report read +4.6 in its first clocked match and
regressed over the next two. The tuned vector found nothing at the control it
was tuned at, and its clocked reading is compatible with the same near-zero
effect. The null-probe skip was given a sequential test declared before it ran
— batches of 4096 games against the razoring head, stopping at an LLR of ±2.94
or at 16,384 games — and accepted H0 after three batches. None of these is kept,
and none but the stored-bound evaluation is shown to be harmful.

The SPSA moved the reduction table's constant term from 0.75 to 0.83 plies, the
quiescence delta margin from 120 to 136, the swap-list threshold from 0 to 6
(which, every piece value being a multiple of ten, also prunes the even
exchanges), the reverse-futility depth margin from 80 to 75 and the
quiet-futility depth margin from 100 to 95, each by at least half its
perturbation step; every other constant moved by less than half of its own.
The whole tuned vector, from which a longer tune can start:

- aspiration radius 49 rather than 50;
- reductions `0.83 + ln(depth) * ln(index) / 2.264` rather than
  `0.75 + ln(depth) * ln(index) / 2.25`, with the history threshold at 5486
  rather than 5461;
- move-count limit `6 + 25 * depth^2 / 16` rather than `6 + 3 * depth^2 / 2`;
- singular margin 34/16 centipawns per ply rather than 2;
- null-move reduction `3 + (depth - 1) / 3` rather than `3 + depth / 3`, one
  more ply per 197 centipawns of excess rather than 200;
- quiet futility `102 + 95 * depth`, 42 less when not improving, rather than
  `100 + 100 * depth` and 40;
- reverse futility `75 * depth`, 37 less when improving, rather than
  `80 * depth` and 40;
- quiescence delta margin 136 and swap-list threshold 6 rather than 120 and 0;
- history bonus `63 * depth^2` capped at 8297 rather than `64 * depth^2` capped
  at 8192;
- the depth limits of move-count, quiet-futility and reverse-futility pruning,
  and the continuation-history weight, unchanged.

## Fixtures that moved

Each patch re-pinned the records its trees moved after probing larger budgets
on both binaries; the commits give every probe.

The storing patch moved three regression scores with their moves:
`win-hanging-queen` 518 to 511, `contain-lone-rook` -543 to -509 and
`developed-open-game` 39 to 22. `starting-style` plays e2e4 rather than d2d4 at
20,000 nodes, as the base itself does from 100,000 to two million. The
open-king-gambit position, in the personality, objective-contract and standard
suites, is searched at 200,000 nodes rather than 100,000: Aggression 100 now
plays the quiet h2h3 up to 120,000 and the forcing c3d5 from 150,000, and every
profile of both binaries plays c3d5 at 200,000. `central-pawn-thrust` accepts
either central push at both profiles, which both converge on c2c4 from one
million nodes; `standard-knight-e4-investment` accepts the d2b1 the base plays
at Aggression 100 from 50,000; and `standard-avoid-equal-queen-trade` lets the
objective profile take the queen trade into a bare-king draw, which ties the
pinned d2h6 at 0. The null-move contract compares its fixtures at depth nine
rather than seven: with fail-lows stored, the table already refutes at depth
seven most of what the probe did (1.5% saving there, 8.4% at depth nine).

Razoring moved five. `knight-e5-investment`'s Aggression 100 investment was a
20,000-node, depth-four choice that the base declines at every budget from
30,000 to 400,000. It is replaced in the personality and sacrifice-gate suites
by `knight-e6-investment`, mined from this series' self-play: Aggression 100
plays Nxe6, a knight for a pawn that d5 wins back, 18 cp below the objective
Nxa6, at 20,000 to 80,000 nodes, and Aggression 0 plays Nxa6 at every budget to
160,000. `rook-lift-pawn-storm`, the suite's only pawn-storm record separating
the profiles, rested on a 20,000-node pawn push of Aggression 100's that both
binaries abandon at 40,000, and razoring moved its objective record too. It is
replaced by `storm-before-castling`, mined the same way: the objective profile
castles and Aggression 100 plays h7h5 toward the king instead at every budget
from 20,000 to 160,000, 35 cp below castling by the objective search. In the
standard suite the objective profile now declines
`standard-knight-d5-investment`, as the base does from 50,000 nodes, while the
attacking profiles still invest; and the default profile prefers c1e3 to c3d5
in `standard-open-king-gambit` from 200,000 to 400,000 nodes, three centipawns
below it by the objective search. The contract's hash of the personality suite
follows each change, and the frozen sacrifice suite's pinned hash, stale since
an earlier commit, is current again.

## Method

Every match used the in-repo `selfplay` arbiter at concurrency 90 on a
96-thread, 48-core host, so each engine had a little over half a core. The
openings were the 2048-position `selective-search-confirmation.epd` book,
played once in each colour, and for further batches the same book with two
seeded random quiet plies. Pair scores were pooled across batches with the
pentanomial statistics of `tools/run_sprt.py`. Every landed patch was rebuilt
from the worktree and checked node-for-node against the binary that was
measured, and the head's release binary is byte-identical to the razoring
build the matches used.

## Reproduction

```sh
cargo build --release --locked --bin jakgro --bin selfplay
python3 tools/run_sprt.py --runner target/release/selfplay \
  --engine target/release/jakgro --baseline-engine <base-6e4c9a3> \
  --candidate-aggression 75 --baseline-aggression 75 \
  --games 4096 --movetime-ms 50 --concurrency 90 --elo0 0 --elo1 5 \
  --openings docs/tuning/data/selective-search-confirmation.epd \
  --pgn artifacts/series/strength.pgn
```

Replace `--movetime-ms 50` with `--time-control 1.0+0.01` for the clocked
channel, or both aggression flags with `0` for the objective one. The
deterministic gates:

```sh
python3 tools/measure_style.py --engine target/release/jakgro --check
python3 tools/measure_style.py --engine target/release/jakgro \
  --suite tests/data/sacrifice-gates.epd --check
python3 tools/measure_acceptance.py --engine target/release/jakgro --check
python3 tools/measure_acceptance.py --engine target/release/jakgro \
  --suite tests/data/standard-acceptance.epd --selected-profile 75 --check
python3 tools/validate_acceptance_contract.py
cargo test --locked
python3 -m pytest tools/tests
```

`tests/data/sacrifice-acceptance-contract.epd` fails its
`acceptance-unsupported-greek-gift` record (h2h3 rather than a2a4) on the base
binary as on the head; this series does not change it.

## Limitations

- Measured at 50 ms per move and at `1.0+0.01` with each engine on about half a
  core; nothing reaches tournament controls.
- Every figure is against one frozen binary on one host, not an absolute
  rating.
- The two replacement personality records were mined from this series' own
  self-play with the razoring search. They describe that search, and were
  checked to 80,000 and 160,000 nodes rather than beyond.
