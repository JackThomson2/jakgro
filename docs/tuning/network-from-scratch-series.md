# A network trained from scratch, and the clock at 10+0.1

**The network is shipped.** The 512-unit network trained from scratch on the
published corpus together with the self-play the head generated with the
tablebases on measures **+25.1 Elo [+13.1, +37.2]** against the network it
replaces at `10+0.1`, Aggression 75 on both sides; the sequential test
`[0, 5]` accepted H1 after 1,316 games, and a fixed 2,048-game confirmation
from a second opening seed measures +24.0 [+14.3, +33.7]. At 50 ms per move
from the random-walk book it measures +6.4 [+0.3, +12.5]: the gain grows with
the search, as the correction history's did. Its forcing-move and check
rates are 1.019 [0.986, 1.053] and 1.037 [0.959, 1.119] of the replaced
network's in the same games. It moves the fixed-node fixtures, and five
profile controls are replaced by positions mined from self-play the network
was not trained on.

The clock was measured above 27 ms per move for the first time. Spreading
the soft budget over more moves with the whole increment added lost at 26
moves and was not shown to gain at 20. Adding the whole increment to the
fifteen-move budget measured +7.3 [+1.4, +13.2] over 4,096 games but stopped
at the game cap with its sequential test short of acceptance, so the clock is
unchanged; that arm is the one worth a confirmation run.

The runs were made on 28 October 2026 against the series head `a30ddd8`; the
confirmations played the commit that ships the network against it, and the
last clock arm was measured on that commit.

| Arm | Games | Wins / draws / losses | Elo [95% interval] | Pentanomial | SPRT `[0, 5]` | Forcing / checks |
| --- | ---: | --- | --- | --- | --- | --- |
| from-scratch network against the shipped one | 1,316 | 335 / 741 / 240 | **+25.1 [+13.1, +37.2]** | 22 / 119 / 301 / 174 / 42 | H1 accepted, LLR +2.95 | 1.019 [0.986, 1.053] / 1.037 [0.959, 1.119] |
| the same, fixed 2,048 games from a second opening seed | 2,048 | 534 / 1,121 / 393 | **+24.0 [+14.3, +33.7]** | 27 / 219 / 426 / 290 / 62 | not sequential | 1.013 [0.985, 1.041] / 1.021 [0.956, 1.089] |
| the same, 50 ms per move, random-walk book (selfplay) | 4,096 | 1,641 / 889 / 1,566 | +6.4 [+0.3, +12.5] | 92 / 249 / 1,311 / 284 / 112 | no decision in the batch, LLR +1.99 | 1.012 / 1.022 |
| clock over 20 moves, whole increment | 4,096 | 764 / 2,604 / 728 | +3.1 [−3.2, +9.3] | 56 / 452 / 1,006 / 468 / 66 | no decision at the cap, LLR +0.27 | 0.989 [0.970, 1.008] / 0.976 [0.933, 1.021] |
| clock over 26 moves, whole increment | 1,878 | 341 / 1,138 / 399 | −10.7 [−20.0, −1.5] | 33 / 230 / 462 / 190 / 24 | H0 accepted, LLR −2.97 | 0.986 [0.958, 1.016] / 0.982 [0.917, 1.053] |
| whole increment, on the new head | 4,096 | 776 / 2,630 / 690 | +7.3 [+1.4, +13.2] | 44 / 389 / 1,113 / 441 / 61 | no decision at the cap, LLR +2.67 | 1.001 [0.981, 1.020] / 1.000 [0.955, 1.047] |

Intervals are pair-normal over complete colour-reversed rounds and are not
adjusted for sequential stopping: an arm the test stops early at H1 carries
the usual upward bias, which is why the network was confirmed with a fixed
number of games. Accepting H0 rejects a gain candidate; it does not prove the
true difference is zero or negative.

## The network

The audit's highest-expected network experiment (its items D1 and D4) was a
corpus the head labels itself, with the tablebases on, and a network trained
from scratch on it with the recipe's later devices on from the first epoch.
The [exclusion, ordering and corpus report](exclusion-ordering-series.md)
measured two continuations of the shipped network on the first 53.5M of the
new rows and recorded that more of the corpus was being generated. The
generation finished, 80 groups of 32,768 games at 20,000 nodes, Aggression 75
against 0 from the random-walk book with 8, 12 or 16 random plies, played and
labelled with the 3-4-5-man tables by the head at `d30d40f` (the first forty
groups) and by the head of 7 October (the rest); and a network was trained
from scratch on it on 9 October. Nothing recorded that it was ever screened.

It is a 512-unit factorized network of the shipped architecture (format 3,
6144 features, eight king buckets, squared clipped ReLU), trained for 80
epochs with full-parameter Adam at rate 0.0057 decaying by 0.9 per epoch,
batch 8192, L2 1e-6, seed 75 and lambda 0.25, with the king-bucket
factorizer, quantization-aware training and an exponential moving average of
the weights (0.9998) on from the first epoch. Its corpus is 333.9M rows after
deduplication: groups 0 to 77 of the new corpus, the published 10,000-node
corpus, and the 20,000-node groups of the two earlier corpora. Groups 78 and
79 of the new corpus and groups 69 to 71 of the earlier one, 6.6M rows, are
the development split; the averaged weights of epoch 75 were selected at a
development label MSE of 0.008717. That figure is not comparable with the
shipped network's 0.008509, which was measured on a different split. The
training report is `nets/jakgro.report.json`.

The screen loaded the network into the series-head binary through `EvalFile`
and played it against the same binary with its embedded network, so the two
sides differ only in the network; their time per move is the same to within
a few milliseconds at every move of the game, 0.63 s on the first move out of
book and 0.19 s at move forty for both. The attacking profile plays no fewer
forcing moves or checks with the new network in these games, which is the
style evidence that matters more than the fixed-node fixtures below: those
sample depth four to ten at their budgets, and the games were played at
depth twelve to fourteen.

## The clock at 10+0.1

The soft budget is the usable time over fifteen moves plus three quarters of
the increment, and the hard limit three times that; an iteration starts only
when twice the last one's time fits in what remains of the soft budget. The
fifteen moves were chosen at `1.0+0.01`, where their neighbours were inside
each other's intervals and where the overheads are a fifth of the budget.
At `10+0.1` the head spends 0.63 s on its first move out of book, 0.38 s at
move twenty, 0.19 s at move forty and 0.11 s at move seventy, and its
completed depth falls from about 14 to 12.4 by move forty before rising again
on the increment; the median game is about 64 moves long and three in five
are drawn.

Three arms were measured. Spreading the budget over 26 moves with the whole
increment moves time from the opening to the endgame: 0.41 s on the first
move out of book against 0.63, the same at move thirty, 0.21 s against 0.19
at move forty and 0.18 against 0.15 at move fifty. It finished its games with
more of its clock unused, 14.3 s spent per game against 15.2, and lost
10.7 Elo. Over 20 moves with the whole increment the move is smaller, 0.51 s
on the first move out of book, the same as the head's at move thirty and
0.21 s against 0.19 at move forty, and the result is +3.1 [−3.2, +9.3] over
the whole 4,096 games, with no decision. The whole increment alone, over
fifteen moves, adds a quarter of the increment to every move's soft budget
and so spends a little more on every move, 0.65 s on the first move out of
book against 0.63 and 15.6 s per game against 15.2; it measured +7.3 [+1.4,
+13.2] over the whole 4,096 games, and its sequential test ended at the cap
with an LLR of +2.67 against the 2.94 that accepts H1.

Moving time from the opening into the endgame lost or did nothing; spending
a little more on every move by adding the whole increment gained about seven
Elo without a decision. The clock is left as it is. A +7 that stops short of
its pre-registered acceptance is what selection noise looks like as often as
a gain, and the change is one line with no fixture to re-pin, so a
confirmation from a second opening seed, or at `60+0.6`, can decide it
cheaply. The three arms also say the fifteen-move divisor chosen at
`1.0+0.01` is not too short at `10+0.1`: both longer divisors spent less in
the opening and gained nothing for it.

## Fixtures that moved

A new network moves the fixed-node fixtures, as every earlier one has.

**Regression records.** Six follow the network: `win-hanging-queen` 497 to
399, `punish-central-queen` 1169 to 1130, `contain-lone-rook` −527 to −431,
`starting-style` 16 to 29, `developed-open-game` 30 to 35, and
`open-game-style` plays Nc3 at 29 instead of Nf3 at 24.

**Profile controls replaced.** Five controls no longer separate the profiles
as intended at their budgets, nor stably at any budget probed between 20,000
and 200,000 nodes. `storm-before-castling` castles at both profiles;
`rook-e6-investment` retreats the bishop at both; Aggression 0 now gives the
exchange in `rook-f3-investment` too; `avoid-queen-trade-queen-c7` trades at
Aggression 100 at 20,000 nodes and plays a third move at 40,000 and 60,000;
and `queenside-pawn-thrust` plays Ne2 at both up to 100,000 nodes. Each is
replaced by a position mined from the development split of the new corpus,
self-play the network was not trained on.

The miner (`mine_fixtures.py`) searched 20,000 positions from groups 78 and 79
at Aggression 0 and 100 with 20,000 nodes and fresh tables; 7,129 of them,
about a third, separated the profiles. The positions were sorted into motifs
by what the two moves are (a capture that loses material by the swap list,
a queen trade declined, a pawn moved at the enemy king's files with the kings
on opposite flanks, a pawn thrust against a piece move), and the verifier
(`verify_fixtures.py`) kept those where each profile chose the same move at
20,000, 40,000, 60,000 and 100,000 nodes; 25 held. Of those, the five below
were chosen. The objective loss is Aggression 0's score less its score
restricted to the Aggression 100 move, at 20,000 and at 100,000 nodes.

| New fixture | Replaces | Position | Aggression 0 | Aggression 100 | Objective loss |
| --- | --- | --- | --- | --- | --- |
| `storm-a5-before-castling` | `storm-before-castling` | `rn2k1nr/1p1b1p2/pq1Pp1p1/3p4/3P1B1p/1PP5/1K2BPPP/R2Q1NR1 b kq - 1 14` | Nc6 | a5, at the king on b2 | −6 / 0 |
| `knight-f2-investment` | `rook-e6-investment` | `rn3r1k/p1b1q1pp/2p5/Np2p3/1P2n1b1/B3QN2/P3BPPP/2R1R1K1 b - - 1 18` | Nf6 | Nxf2, then Bb6 pinning the queen | −55 / −3 |
| `knight-d5-pawn-investment` | `rook-f3-investment` | `rn3kr1/p4pp1/1pp2b2/1P1p3q/8/2NPQN1b/PBP5/R2K3R w - - 0 18` | Kc1 | Nxd5, a knight for a pawn | 90 / 50 |
| `avoid-queen-trade-queen-a4` | `avoid-queen-trade-queen-c7` | `r3kr2/p7/2pq2p1/R2pbbP1/1Q2np2/2P2B2/1P2PP2/1NB1K2R w K - 6 20` | Qxd6, trading queens | Qa4 | −3 / −27 |
| `queenside-pawn-thrust-b4` | `queenside-pawn-thrust` | `1r2k2r/p1pp1ppp/1pn2n2/2b1p2q/4P3/1PP2PP1/P1NPQN1P/R1B1K2R w Kk - 3 12` | Bb2 | b4, against the bishop on c5 | −5 / 29 |

A negative loss means the restricted search scored the attacking move above
the move the unrestricted search chose, which happens when the two searches
complete different depths; the sacrifice of `knight-f2-investment` is not
worse by the engine's own measure, and the profiles still separate on it at
every budget. The sacrifice gates and the objective/personality contract
mirror the two sacrifice replacements and the simplification replacement.

**Moves accepted beside the old ones.** Where both profiles now agree on
another move, the move is added to the accepted set: Nc3 in
`early-king-pressure`, Nc6 in `black-king-pressure`, and Be3 in
`open-king-gambit`, in the personality suite and in the objective/personality
contract. The last is the record the sixth series refused to move for an
ordering device: with this network both profiles play Be3 at 200,000 nodes,
and Aggression 100 plays it at every budget from 100,000 to 800,000, while
its search restricted to the knight investment Nd5 scores the investment a
few centipawns higher (141 against 134 at 200,000 nodes) and Aggression 0
plays Nd5 from 400,000 nodes. This series accepts the change because the
network gains twenty-five Elo while playing more forcing moves and checks in
games, where the ordering device gained two at most.

**The standard-profile suite** accepts the moves the profiles now play, each
inside its loss cap: Aggression 0 plays Bc4 in `standard-central-space` and
Be3 in `standard-open-king-gambit`, where Aggression 100 plays Be3 as well;
Aggression 75 plays h3 in `standard-opposite-castle-storm` at a loss of 4 of
45, the investment Nd2–e4 itself in `standard-knight-e4-investment` at 18 of
120, and Qe2 in `standard-avoid-equal-queen-trade`, declining the trade, at
0.

**The null-move contract** measured the node saving of null pruning at depth
nine on the contract position where it fires most, the start position at
Aggression 0, and asked for five percent. With this network null pruning
searches 7.8% *more* nodes there than without it; the replaced network saved
6.6% at depth nine but searched 19.8% more at depth ten. At these tree sizes
the ratio swings by a fifth from one depth to the next, whichever network is
loaded:

| Depth | 8 | 9 | 10 | 11 | 12 | Sum |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| replaced network | −32.7% | −6.6% | +19.8% | −12.8% | −15.0% | −10.3% |
| shipped network | −16.8% | +7.8% | +15.2% | +13.9% | −26.6% | −7.8% |

The contract now sums the nodes of fresh searches at depths eight to twelve,
which both networks pass with the same five-percent requirement. It costs
time: the test runs in 72 s unoptimised on this loaded host, against 13 s
before.

**Not changed.** The frozen sacrifice acceptance contract
(`sacrifice-acceptance-contract.epd`), which no CI gate runs, already failed
on the series head (`acceptance-unsupported-greek-gift` at a loss of 3 against
a cap of 0). With this network Aggression 100 also misses the bishop
sacrifice of `acceptance-bishop-f7-sacrifice` at 20,000 nodes, where its
search completes only depth four; it plays the sacrifice from 40,000 nodes
on. The suite is left as it was.

## Match setup and accounting

Every fastchess arm used `alpha 1.8.2 20260912-60d7a7a` with Threads 1,
Hash 16 MiB, Aggression 75 and no tablebases, `8moves_v3.pgn` with sixteen
opening plies in random order and seed 14875 (31337 for the network's
confirmation), colours reversed for every opening, a cap of 2,048 rounds,
draw adjudication `movenumber=40 movecount=8 score=10`, resignation
`movecount=3 score=600 twosided=true`, at most 200 moves and a time margin
of 100 ms. The network and the first two clock arms ran at the same time at a
concurrency of 28 each, 84 games on the 96 logical CPUs; the later runs
overlapped one another at up to 80 games at once. The 50 ms run used the
selfplay arbiter through `tools/run_sprt.py` instead, with the random-walk
book (`selective-search-confirmation.epd`, 2,048 colour-reversed pairs),
Hash 16 and a concurrency of 30; its interval is that tool's pair-normal one
and its style ratios are per-move rates from `tools/analyze_match.py`,
without a bootstrap. All binaries were built with Rust 1.98.1, the release
profile, locked and offline dependencies and no extra `RUSTFLAGS`.

Fastchess writes the PGNs in completion order without FEN headers for a PGN
book, so the paired analysis (`analyze_pairs.py`, the analyzer of the LMR
and evasion-ordering screens, copied unchanged) pairs games by their Round
header and checks reversed colours and matching opening prefixes. Every
analysis reproduces the runner's W/D/L and pentanomial counts. Every runner
exited zero, every recorded termination was normal or adjudication, and the
logs contain no error, stall, disconnection or forfeit. The two clock arms
that ran to the cap logged fastchess warnings that a reported principal
variation continued past a threefold repetition, 4 in the 20-move arm (all
from the unchanged baseline) and 22 in the increment arm (15 from the
candidate, 7 from the baseline); they concern the reported line, not the
moves played. The two arms the sequential test stopped left complete but
unpaired rounds, 4 in the network arm (656, 658, 661 and 662) and 3 in the
26-move arm (940, 943 and 951), all with normal or adjudicated
terminations; they are kept in the PGNs and excluded from the paired
estimates.

## Reproduction

The [data directory](data/network-from-scratch/) holds the
[protocol](data/network-from-scratch/protocol.json), the
[binary, network, book and runner hashes](data/network-from-scratch/builds.json),
each arm's compressed PGN, runner log, saved configuration and paired
analysis, the analyzer and its tests, and the fixture-mining tools with
their raw and verified output (`mined-round1.jsonl.gz`,
`mined-round2.jsonl.gz`, `verified.jsonl.gz`).

With `FASTCHESS`, `BOOK`, `CANDIDATE`, `BASELINE`, `OUT` and `CONCURRENCY`
set for the local machine, each arm is

```sh
"$FASTCHESS" \
  -engine cmd="$CANDIDATE" name=Candidate $CANDIDATE_OPTIONS \
  -engine cmd="$BASELINE" name=Baseline \
  -each proto=uci tc=10+0.1 option.Threads=1 option.Hash=16 \
    option.Aggression=75 timemargin=100 \
  -rounds 2048 -games 2 -repeat \
  -openings file="$BOOK" format=pgn order=random plies=16 \
  -srand 14875 -concurrency "$CONCURRENCY" \
  -draw movenumber=40 movecount=8 score=10 \
  -resign movecount=3 score=600 twosided=true -maxmoves 200 \
  -sprt elo0=0 elo1=5 alpha=0.05 beta=0.05 model=logistic \
  -pgnout file="$OUT.pgn" append=false \
  -config outname="$OUT.config.json" \
  -autosaveinterval 200 -ratinginterval 200 > "$OUT.match.log" 2>&1
```

where the network arm sets `CANDIDATE` and `BASELINE` to the same series-head
binary and `CANDIDATE_OPTIONS` to `option.EvalFile=<path>/network.nnue`, the
confirmation sets `CANDIDATE` to a binary of the commit that ships the
network, `-rounds 1024`, `-srand 31337` and no `-sprt`, and the clock arms
leave `CANDIDATE_OPTIONS` empty. The 50 ms run is

```sh
python3 tools/run_sprt.py --runner "$SELFPLAY" --engine "$CANDIDATE" \
  --baseline-engine "$BASELINE" --candidate-aggression 75 --baseline-aggression 75 \
  --games 4096 --movetime-ms 50 --concurrency 30 --elo0 0 --elo1 5 \
  --openings docs/tuning/data/selective-search-confirmation.epd --pgn net-50ms.pgn
```

The analyses are reproduced without rerunning games:

```sh
DATA=docs/tuning/data/network-from-scratch
PYTHONDONTWRITEBYTECODE=1 python3 -m pytest -q "$DATA/test_analyze_pairs.py"
python3 "$DATA/analyze_pairs.py" "$DATA/net-g3scratch.pgn.gz" --allow-incomplete-pairs
python3 "$DATA/analyze_pairs.py" "$DATA/net-confirm.pgn.gz" --seed 31337
```

and the fixture candidates from a release binary of the shipped head and the
corpus groups:

```sh
python3 "$DATA/mine_fixtures.py" "$ENGINE" mined.jsonl \
  artifacts/gen3/n20000/g78-s4624-4631-p8.txt \
  artifacts/gen3/n20000/g79-s4632-4639-p12.txt --slice 0/4 --limit 1500
python3 "$DATA/verify_fixtures.py" "$ENGINE" sacrifice verified.jsonl mined.jsonl
python3 "$DATA/probe_fixtures.py" "$ENGINE" tests/data/personality.epd open-king-gambit \
  --profiles 0,75,100 --budgets 100000,200000,400000,800000 --restrict c3d5
```

The mining tools need python-chess. These are short-control relative results
against one baseline on one host, not an absolute rating.
