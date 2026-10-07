# Keyed exclusion searches, ordering updates, and the regenerated corpus

This report records the sixth round taken from the
[Elo opportunity audit](elo-opportunity-audit.md): its items S12 and O1 on the
search side, and the first measurements of the corpus regeneration its item
D1 asked for. The series head was `c8e3e2e`; every match below is against
that head at Aggression 75 on both sides, 4096 paired games from the
random-walk book, Hash at its 16 MiB default. Style ratios are the
candidate's forcing-move and check rates per move against the baseline's over
the same games, with a bootstrap over the 2048 colour-reversed opening pairs.

## Exclusion searches keyed by the excluded move (audit S12)

A singular exclusion search, the reduced search of a node with its hash move
left out that decides whether that move is extended, probed the table under
the node's own key. It could neither cut on what it found, since the node's
entry names the excluded move and scores the position with it, nor store
what it settled, since its result is not the position's value; so every
iteration re-expanded it, and the second strength series had measured the
exclusion searches at 10.7–30.6% of the nodes. Stockfish probes them under
the position's key mixed with the excluded move, so that they never read the
position's own entry, but stores nothing for them either.

Two forms were built. Both mix the excluded move into the probe key and let
the exclusion search cut on an entry stored under that key; the first stores
nothing at exclusion nodes, as Stockfish does, so its mixed-key probe can hit
only the depth-0 evaluation entry the node itself writes, while the second
also stores the exclusion search's own result, so the exclusion search of a
later iteration at the same node and move can be answered from the table.
In both, the exclusion node no longer sees the parent's hash move, so
internal iterative reduction applies to it, and no longer reads the parent's
cached evaluation.

| Form | Nodes at depth 13, Aggression 75 / 0 | 50 ms | 10+0.1 | Forcing / checks at 10+0.1 |
|---|---|---|---|---|
| mixed key, no store at exclusion nodes | 21,329,658 (+0.6%) / 18,937,033 (−5.2%) | +4.7 [−1.5, +10.8] | −2.7 [−7.9, +2.4] | 1.003 [0.986, 1.020] / 1.011 [0.966, 1.060] |
| mixed key, exclusion results stored | 22,648,525 (+6.8%) / 17,916,908 (−10.4%) | +2.5 [−3.4, +8.5] | +1.6 [−3.4, +6.6] | 1.004 [0.985, 1.025] / 1.013 [0.964, 1.067] |

Neither form clears zero at 10+0.1, and the 50 ms readings are inside the
noise of that control; pooled over both controls the stored form reads about
+2.0 ± 3.9 and the unstored form +1.0 ± 4.0. The audit's premise was that
re-expanding the exclusion searches wasted a measurable share of the tree;
the node counts say the mixed key saves 5–10% of the depth-13 tree at
Aggression 0 and nothing at the default profile, where the exclusion node's
lost access to the parent's cached evaluation and its new internal
iterative reduction cost as much as the reuse saves. Not kept. A form that
hands the parent's evaluation to the exclusion search, so that only the
reuse remains, is the one variant left untried.

## Ordering updates on quiet cutoffs and table cutoffs (audit O1)

On a quiet cutoff the search penalised the quiets searched before it and
rewarded the cutting move, but the captures searched before it, which are
the good captures the picker leads with, learned nothing: capture history
was updated only when a capture cut. A quiet hash move that settled the node
from a stored lower bound earned nothing either, although it had cut there
as surely as if it had been searched; Stockfish credits both (about two Elo
each by its accounting).

Two devices were built: the captures searched before a quiet cutoff receive
the malus a capture cutoff would have given them, and a quiet hash move that
cuts on a table bound at or above beta receives the cutoff bonus and the
killer slot. They were matched together first and then the malus alone,
after the fixtures showed the table-cutoff bonus to be the part that moved
the Aggression 100 choice in `open-king-gambit` from the forcing c3d5 to the
quiet a2a3 at every budget from 100,000 to 800,000 nodes, a change of the
style contract the series was not willing to make for a device Stockfish
credits with two Elo.

| Form | Nodes at depth 13, Aggression 75 / 0 | 50 ms | Forcing / checks at 50 ms | 10+0.1 | Forcing / checks at 10+0.1 |
|---|---|---|---|---|---|
| both devices | 23,621,975 (−0.1%) / 20,149,311 (+0.8%) | +6.5 [+0.4, +12.7] | 1.001 [0.984, 1.019] / 0.995 [0.951, 1.041] | +1.7 [−3.2, +6.6] | 1.014 [0.996, 1.033] / 1.034 [0.986, 1.085] |
| failed-capture malus alone | 22,498,080 (−4.8%) / 20,003,504 (+0.1%) | +4.5 [−1.3, +10.3] | 1.001 [0.983, 1.019] / 1.006 [0.962, 1.052] | −3.1 [−8.1, +1.8] | 0.993 [0.976, 1.010] / 0.985 [0.939, 1.031] |

Neither form is kept. The malus alone pools to about +0.7 ± 3.9 over the
two controls, nothing. Both devices together pool to about +4.1 ± 3.9, with
the 10+0.1 reading inside the noise, and the difference between the two
forms says the table-cutoff bonus carries whatever gain there is; but that
is the device that makes the maximal profile give up the forcing knight
investment in `open-king-gambit` at every budget probed, while the objective
profile keeps playing it and the restricted searches of the two moves still
score the investment nine centipawns higher. A borderline gain bought with a
contract change at the attacking profile is the kind of result the audit's
methodology section warns is most often a winner's curse, so the series
records both and keeps neither. The Stockfish form of the table-cutoff bonus
also updates the continuation history and penalises the previous ply's
early quiets; a future attempt should carry those and be judged at 10+0.1
with the style contract intact.

## The regenerated corpus (audit D1)

The audit's highest-expected network item was to regenerate the training
corpus with the current engine as teacher and the Syzygy tables on both
sides, replacing the 131M rows that the run-42 128-unit network had labelled
from 10,000-node searches. `generate_nnue_corpus.py` gained the tablebase
passthrough, and forty groups of 32,768 games at 20,000 nodes, Aggression 75
against 0 from the random-walk book with 8, 12 or 16 random plies, were
generated by the head at `d30d40f` with the 3-4-5-man tables: 53.5M rows in
about nine hours at a concurrency of 24 to 44. 0.6% of the rows carry a
tablebase-decided score, concentrated at six and seven pieces, and 8% a
score of 800 or more that is not.

Two continuations of the shipped network were trained on the new rows, eight
epochs each with the shipped recipe (learning rate 1e-4 decaying by 0.9,
batch 8192, λ 0.25, factorizer, quantisation-aware training, EMA 0.9998), and
screened against the embedded network on the same engine over 4096 paired
games at 50,000 nodes per move.

| Corpus | Rows after deduplication | Development label MSE (shipped 0.008506) | Screen, tables on both sides | Screen, tables off |
|---|---|---|---|---|
| new rows + the 20,000-node groups, the 10,000-node rows dropped | 156.0M | 0.008386 | −6.4 [−12.0, −0.5] | −6.4 [−12.3, −0.2] |
| new rows + the whole shipped corpus | 285.9M | 0.008374 | +3.3 [−2.8, +9.4] | +2.4 [−3.6, +8.6] |

Replacing the 10,000-node rows with the new ones lost six Elo whichever way
it was screened, so the tables-on screen was not masking an endgame gain:
the corpus had shrunk by a third and the development labels, which are the
old teacher's, had improved only because the network agreed more with them.
Adding the new rows to the whole shipped corpus instead gains about three:
+3.3 [−2.8, +9.4] at 50,000 nodes with the tables on, +2.4 [−3.6, +8.6]
without, and +4.4 [−2.1, +10.6] at 50 ms, about +3.4 ± 3.6 pooled over the
12,288 games, with the forcing and check rates at 50 ms up by two and five
percent (1.020 [1.004, 1.038] and 1.052 [1.007, 1.101]). The difference
between the two continuations, ten Elo for the same new rows, says the
volume of data mattered more than the labels' source at this scale.

The additive network is not shipped. Shipping a network re-pins every
fixed-node fixture, and a pooled three Elo at the edge of its interval is
not worth that until more of the corpus is the head's. Forty more groups are
being generated by the current head with the tables on, which would put the
head-labelled share near 40%; the audit's from-scratch schedule (D4) on that
corpus, screened against the shipped network at 50,000 nodes and 50 ms and
then at 10+0.1, is the next network experiment.

## Fixtures that moved

None: no candidate was kept, so the suites are as the series head left them.
The probes that decided the ordering question are worth recording, since
they show what the fixtures measure: with both ordering devices the
Aggression 100 search of `open-king-gambit` returns a2a3 at 112 centipawns
at every budget from 100,000 to 800,000 nodes, while the same search
restricted to c3d5 returns 117 at a deeper completed depth, so the
unrestricted search never resolves the better move; the failed-capture
malus alone leaves c3d5 in place at every budget.
