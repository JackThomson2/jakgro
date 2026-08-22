# Verified null-move pruning result

This report validates the null-only engine series ending at `03609acb6414781e684c3a1dd87a3bfddd096cd7` against the accepted engine `bc51beaa7e13ea47a9090168589e788984a90da7`. Evaluation, root personality, sacrifice verification, LMR, SEE, and all Aggression behavior are unchanged; the candidate adds always-verified null-move pruning plus diagnostic telemetry and A/B controls.

Machine-readable evidence:

- [`data/verified-null-gate.json`](data/verified-null-gate.json)
- [`data/verified-null-strength.summary.json`](data/verified-null-strength.summary.json)
- [`data/verified-null-benchmark.csv`](data/verified-null-benchmark.csv)

## Acceptance result

| Gate | Requirement | Observation | Result |
| --- | ---: | ---: | --- |
| Frozen personality choices | 16/16 | 16/16 | Pass |
| Frozen sacrifice/control choices | 4/4 | 4/4 | Pass |
| Same-profile smoke Elo | At least -35 | 0.0 | Pass |
| Forcing-rate retention | At least 90% | 98.81% | Pass |
| Null-on/null-off objective equivalence | Identical | Identical on all contracted positions | Pass |
| Null node reduction | At least 5% | 18.01% geometric mean | Pass |
| Null activity in forbidden contract classes | None | None | Pass |

The null-only candidate passes the preregistered screening gate. This result validates the search optimization; it does not alter or supersede the personality and sacrifice claims in [Verified aggression: style and strength result](verified-aggression-elo.md).

## Same-profile strength screen

Candidate and baseline both used `Aggression=100`, 10,000 nodes per move, and 48 sequential color-reversed opening pairs:

- candidate W/D/L: **39/18/39**;
- candidate score: **50.00%**;
- Elo point estimate: **0.0**;
- 95% Hoeffding Elo interval: **-143.91 to +143.91**;
- decisive games: **81.25%**;
- average game length: **76.13 plies**.

Every opening pair produced exactly one point for each engine: the pair distribution was `{"1.0": 48}`. The 96-game run is a non-inferiority smoke screen, not evidence of an Elo gain. Its wide confidence interval does not establish superiority; it establishes that the point estimate clears the preregistered -35 Elo screen while all deterministic and performance gates pass.

## Personality retention

The candidate and accepted baseline selected the same move in all 16 fixed-node personality positions and all four sacrifice/control positions. No safety, anti-sacrifice, initiative, king-attack, pawn-storm, simplification, or sacrifice choice changed.

Complete-game forcing rates were also stable:

| Indicator per 100 moves | Candidate | Baseline |
| --- | ---: | ---: |
| Checks | 15.35 | 15.85 |
| Captures | 22.03 | 21.98 |
| Promotions | 0.44 | 0.47 |
| Checks, captures, or promotions | 34.15 | 34.56 |

The combined forcing rate retained **98.81%** of the accepted engine, comfortably above the 90% floor. Evaluation and root-personality policy are unchanged, and the fixed-node plus complete-game gates show that the integrated null search retained the existing aggressive behavior.

## Null correctness policy

A null probe is eligible only when all of the following hold:

- normal legal search mode rather than synthetic-null or verification mode;
- depth at least four;
- a non-PV null window;
- side to move not in check;
- no mate-bound window;
- halfmove clock below 99;
- objective static evaluation at least beta;
- a rook or queen, or at least two minor pieces;
- `cozy-chess` can construct the null board.

The synthetic branch:

- is not pushed into real repetition history;
- ignores synthetic repetition and rule-fifty claims;
- preserves checkmate, stalemate, and dead-material detection;
- reads or writes no TT entry;
- updates no killer or history score;
- cannot recursively make another null move.

Every null fail-high is then re-searched from the original legal board at reduced depth with normal draw semantics, null disabled, and no ordering side effects. Only a verification that also reaches beta returns a fail-hard cutoff. Probe and verification nodes count toward the ordinary node limit, and null moves never appear in the PV.

## Fixed-depth search benefit

Null enabled and disabled searches returned identical objective moves, scores, and legal PVs on every frozen null-safety contract position. Eligible benchmark rows produced:

| Position | Null off nodes | Null on nodes | Reduction | Attempts | Cutoffs |
| --- | ---: | ---: | ---: | ---: | ---: |
| Win hanging queen | 7,164 | 5,021 | 29.91% | 6 | 6 |
| Punish central queen | 227,284 | 203,112 | 10.64% | 31 | 29 |
| Starting style | 42,899 | 38,245 | 10.85% | 13 | 13 |
| Open-game style | 103,045 | 85,167 | 17.35% | 23 | 22 |
| Developed open game | 103,568 | 83,128 | 19.74% | 23 | 22 |

The geometric-mean node reduction over active rows was **18.01%**. Mate-in-one and lone-rook defensive rows correctly recorded no null attempts and no node change.

## Identity and reproducibility

- candidate commit: `03609acb6414781e684c3a1dd87a3bfddd096cd7`;
- baseline commit: `bc51beaa7e13ea47a9090168589e788984a90da7`;
- candidate binary SHA-256: `08112f80df8612b5c02cfb0a8ce3bb90391cfa31407a03a62a97112985b15e0f`;
- baseline binary SHA-256: `59ac6b9896fb80d1556623aae84ae64532185188766706926bbcf06825ea8a70`;
- opening SHA-256: `8f67f7bdb3c659140516e9f692694ca8513633ee0d9302374dccef927eaa0cde`;
- `cutechess-cli 1.5.1`, SHA-256 `bb8ec8df71ce0ef95ec03614440fe93c31730bbc0c8fbfd07a535e14b7b5d550`;
- 96 games, 48 pairs, 10,000 nodes per move, 16 MiB hash, one concurrent game;
- PGN SHA-256: `0b863a44224c09923c5e34fb56cde62178aea9b302f97706fa5c1bf63ceb9ffb`;
- manifest SHA-256: `75c20b5885c54ab632815d9b8a827e204c53baf6556ba42ec2128e50ce204585`;
- analyzer summary SHA-256: `35697c96a69eea60ce34a20cb812eb0fa50336c8bae5bb3db23a97912abb2716`;
- fixed-depth benchmark SHA-256: `285afbfdd330889a8dc9ca16e2670976a1161345bb05feaab2be8aced0d52cd4`.

## Limits

The strength result is one 96-game fixed-node self-play screen with a wide interval, not an SPRT or external rating. Raw PGN and manifest files are represented by hashes and the committed analyzer summary rather than tracked directly. Benchmark timings are single-machine observations; deterministic objective equivalence and node counts are the stronger evidence. A larger external or tournament-time-control test is still required before claiming an Elo gain.
