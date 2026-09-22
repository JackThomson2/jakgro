# Table, repetition and clock series

## Verdict

Six patches landed out of twenty-three candidates measured, twenty-five
binaries counting the clock divisor's neighbours. Measured head against the
shipped base (`a7ec76e`, the 512-unit network engine) over 4096
colour-reversed games at a fixed 50 ms per move, the series is worth
**+42.5 Elo [36.4, 48.7]** at Aggression 75 and **+31.6 Elo [25.9, 37.4]** at
Aggression 0, both accepting H1 with no faults. Under the clock at
`1.0+0.01`, where the fifth patch also acts, 4096 games measure
**+48.8 Elo [42.3, 55.2]**, accept H1.

The engine did not become duller for it. Over 4096 Aggression 75 versus 0
games at 50 ms the head plays 29.26 forcing moves per hundred against
26.25 for its objective profile, a ratio of 1.115 against the base's 1.088
on the same match, and 12.17 checks per hundred against 8.46. The measured
cost of the default profile against objective play is -35.1 Elo [-41.5,
-28.7] on the head against -40.0 Elo [-46.5, -33.5] on the base.

## Why this series

Every previous series tuned what the search does with its nodes: which moves
it prunes, reduces or extends, and what the evaluation says about the leaves.
This one looked instead for places where the search threw away what it
already knew. It found four, and each was worth more than any pruning rule
measured beside it: a repetition inside the tree was not scored as a draw
until its third occurrence; the results of every subtree whose best line
ended in a repetition were withheld from the table; a principal-variation
node stopped at a table hit and reported the stored line; and two cutoffs
that knew a bound returned beta. The fifth found the clock leaving a third
of every soft budget unspent, and the fourth found the default profile
paying seventy-five centipawns of reverse-futility margin for no forcing
play that could be measured.

The pattern in the rejections is as clear as the pattern in the acceptances.
Every change that let the search *trust* the table more — storing bounds
after moves were pruned or reduced without re-search, or reading a stored
bound in place of the static evaluation for pruning — lost about nineteen
Elo, twice. Every change that let the search *see* more of what it had
already searched — the repetition, the withheld results, the searched-through
PV node, the fail-soft bounds — gained. The table is a record of what was
searched, and this engine's pruning is aggressive enough that a stored bound
is only as good as the search that produced it.

## Per patch

Each row is the patch against its immediate parent at Aggression 75 on both
sides, 4096 games at 50 ms per move unless the row says otherwise.

|Patch|Elo against parent|
|---|---|
|Repetition inside the tree scored as a draw on its first return|**+16.3 [10.7, 21.9]**|
|Path-dependent results stored in the table|**+8.0 [2.3, 13.6]**|
|Principal-variation nodes searched past a table hit|**+12.0 [6.1, 18.0]**|
|Reverse-futility margin without the aggression term at the default|**+10.0 [3.7, 16.3]**|
|Clock budgeted over fifteen moves rather than thirty, at `1.0+0.01`|**+18.8 [12.6, 25.1]**|
|Fail-soft scores from reverse-futility and null-move cutoffs, two matches|+4.7 [-1.6, 10.9], **+11.0 [4.7, 17.4]**|

The clock patch was measured against its neighbours as well: twenty moves
measured +13.8 [7.6, 20.1] and twelve +12.2 [5.8, 18.6] against the same
parent, so fifteen is where the response turns. The fail-soft patch was
measured twice because its first match did not settle it; the two matches
together are about +7.9 over 8192 games.

The reverse-futility patch touches a personality knob, so its style channel
was measured before it was kept: over 2048 Aggression 75 versus 0 games at
50 ms the forcing-move ratio was 1.106 without the term, against 1.088 for
the head that had it, and the cost of the default profile against objective
play was -37.1 Elo [-45.9, -28.4], against the -40.0 recorded for the
previous head. The wild endpoint keeps the margin it had, because at twenty
thousand nodes its knight investment and its refusal of the queen trade are
found in exactly the nodes the narrower margin prunes, and both are controls
the profile suite requires.

## Rejected directions

Each was measured against the head of the moment at Aggression 75, 4096
games at 50 ms per move unless the row says otherwise.

|Candidate|Elo|
|---|---|
|Hash move ordered first in quiescence|-4.9 [-10.9, +1.1]|
|Continuation history at full weight with a follow-up table|+1.3 [-4.7, +7.2]|
|The same with fractional, history-scaled reductions|-1.1 [-7.2, +5.0]|
|Store fail-low and exact results after pruning or reducing moves|**-19.3 [-25.3, -13.3]**|
|Network score scaled by material left, four fifths on a bare board|+3.7 [-2.4, +9.8]|
|The same, seven tenths on a bare board|-5.9 [-12.4, +0.7]|
|Stored bound read in place of the static evaluation for pruning|**-19.1 [-25.6, -12.6]**|
|Countermove ordered after the killers|-1.6 [-8.1, +4.8]|
|King-zone moves reduced like other late quiets|+1.4 [-4.7, +7.6]|
|ProbCut from depth five, margin 200, four plies shallower|-3.2 [-9.4, +3.0]|
|Check extensions capped at two per line at the default, two matches|+5.5 [-0.8, +11.8], -0.8 [-7.0, +5.3]|
|Reductions allowed to leave one ply rather than two, two matches|+5.6 [-0.6, +11.8], +1.4 [-4.7, +7.4]|
|Attacking pawn pushes reduced and pruned like other quiets at the default|-0.8 [-7.2, +5.6]|
|Root moves ordered by the previous iteration's scores|-7.1 [-13.5, -0.8]|
|Volatile searches extended past the soft budget repeatedly, at `1.0+0.01`|-9.2 [-15.4, -3.1]|
|Reverse futility to depth nine and quiet futility to depth eight|-2.3 [-8.4, +3.9]|
|Quiescence checks generated only above the default profile|-2.0 [-8.2, +4.3]|

Two of these, the check-extension cap and the one-ply reduction floor, read
positive in their first match and neutral in their second; over their 8192
games each is worth about +2 to +3 Elo with an interval that includes zero,
and neither is kept.

## Fixtures that moved

The deeper and differently shaped trees at the fixture node budgets moved a
handful of records, each re-pinned in the patch that moved it.
`win-hanging-queen`, `contain-lone-rook`, `punish-central-queen` and
`starting-style` keep their moves at new scores. `rook-lift-pawn-storm` had
both profiles playing g4g5 at 20,000 nodes; the objective profile now prefers
the king move at +177 to the pawn push whose principal variation was a shuffle
the old search could not price, and the attacking profile keeps g4g5 at every
probed budget, so the fixture now separates the profiles.
`avoid-queen-trade-bishop-d4` is a knife-edge 20,000-node choice at Aggression
100 between two moves that both keep the queens on, and accepts either;
`developed-open-game` alternates between three developing moves as the search
changes and is pinned to the 20,000-node choice. The warm-table regressions
asserted that a second search stops at the first search's entries; a
principal-variation node now searches through them, so those tests keep what
they were written for — a tail, the same move, and a score within ten
centipawns — and no longer require the warm variation to be a prefix of the
cold one or the warm tree to be smaller.

## Reproduction

```sh
cargo build --release --locked --bin jakgro --bin selfplay
python3 tools/run_sprt.py --runner target/release/selfplay \
  --engine target/release/jakgro --baseline-engine <base-a7ec76e> \
  --candidate-aggression 75 --baseline-aggression 75 \
  --games 4096 --movetime-ms 50 --concurrency 52 --elo0 0 --elo1 10 \
  --openings docs/tuning/data/selective-search-confirmation.epd \
  --pgn artifacts/series/strength.pgn
```

Swap both aggression flags to `0` for the objective channel, or replace
`--movetime-ms 50` with `--time-control 1.0+0.01` for the clocked
confirmation. The deterministic gates are unchanged:

```sh
python3 tools/measure_style.py --engine target/release/jakgro --check
python3 tools/measure_style.py --engine target/release/jakgro \
  --suite tests/data/sacrifice-gates.epd --check
python3 tools/measure_acceptance.py --engine target/release/jakgro --check
python3 tools/validate_acceptance_contract.py
cargo test --locked
```

## Limitations

- Measured at 50 ms per move and at `1.0+0.01` on a 48-core host shared
  with a corpus generation running at forty games, so each engine had about
  six tenths of a core; the pairing keeps both sides equally slowed. Nothing
  reaches tournament controls.
- The clock patch was measured at `1.0+0.01` only. Fixed-time and
  fixed-node searches are unaffected by it.
- Every strength figure is against one frozen binary on one host, not an
  absolute rating.
