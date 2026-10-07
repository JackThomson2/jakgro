# Quiet evasions in quiescence and the null-move verification depth

This report records the fifth round taken from the
[Elo opportunity audit](elo-opportunity-audit.md), its items Q3 and S4. Both
were rejected; the report exists so that neither is tried again in the same
form. The
series head was `e6f2678`, which carries the late-move-reduction, ordering
and correction-history patches of the earlier series; every match below is
against that head at Aggression 75 on both sides, 4096 paired games from the
random-walk book, Hash at its 16 MiB default. Style ratios are the
candidate's forcing-move and check rates per move against the baseline's over
the same games, with a bootstrap over the 2048 colour-reversed opening pairs.

## Quiet evasions in quiescence (audit Q3)

A quiescence node in check generated and searched every legal evasion. The
first-ply quiet checks every profile searches, and the checks that reach
quiescence once the extensions are spent, therefore cost a full node each on
the defending side: every king walk and every interposition was searched
although the first non-losing evasion already settles the node for a search
that only wants to know whether the check refutes the line. Stockfish 15
stopped after the second quiet evasion once some evasion had scored above
the mated band, captures excepted.

The patch does the same: a node in check searches every capture and, once an
evasion has scored above the mated band, at most two quiet evasions. The
depth-13 book totals move from 21,205,941 to 22,491,578 nodes at Aggression
75 and from 19,985,482 to 20,147,156 at Aggression 0; the trees change
rather than shrink, because the evasions a node no longer searches change
its fail-soft score and so the windows above it.

| 50 ms | Forcing / checks at 50 ms | 10+0.1 | Forcing / checks at 10+0.1 |
|---|---|---|---|
| **−34.1 [−40.4, −27.9]** | 1.037 [1.018, 1.055] / **1.162 [1.108, 1.217]** | not run | — |

Rejected without a second match. The check rate rose by sixteen percent and
the engine lost thirty-four Elo: when the defender's quiet evasions are cut
after two, a check whose refutation is the third evasion scores as if it
had none, the first-ply quiet checks every profile searches in quiescence
look stronger than they are, and the search plays them. Stockfish could
afford the rule because its evasions are ordered by a richer history and its
quiescence searches no quiet checks past the first ply; this engine's
quiescence is built around those checks, and the rule removes exactly the
replies that keep them honest. Any future form would have to exempt the
nodes reached through a quiet check, which is most of the nodes in check
that quiescence sees.

## Null-move verification depth (audit S4)

A null-move fail-high at depth twelve or more was verified by a re-search at
`depth − R` in a mode that forbids null moves for both sides in the whole
subtree. The threshold was chosen in the selectivity series at 50 ms, where
depth twelve is never reached, so the rule had never fired in a match that
accepted or rejected it; at 10+0.1 the search reaches depth seventeen and
the audit's depth-14 telemetry counted 1,122 verifications over forty
positions. Stockfish verifies from depth sixteen and forbids the null move
only for the side to move and only for three quarters of the remaining
depth.

Two settings were matched at 10+0.1 only, since both are inert at 50 ms.

| Setting | 10+0.1 | Forcing / checks at 10+0.1 |
|---|---|---|
| verification from depth 16 | −1.4 [−6.3, +3.6] | 1.001 [0.984, 1.020] / 1.012 [0.965, 1.061] |
| no verification | +1.2 [−3.7, +6.1] | 0.985 [0.967, 1.003] / 0.964 [0.917, 1.013] |

Neither clears zero. Verification from depth twelve costs nothing
measurable against verifying later or not at all at 10+0.1, so the rule the
selectivity series chose at 50 ms stands; the audit's expectation of up to
six Elo from a later or partial verification is not borne out at this
control, and the question is closed unless a 60+0.6 match, where the search
reaches depth twenty and the verification band is wider, reopens it. The
partial form, forbidding the null move only for the side to move and only
for part of the remaining depth, was not built: with both bracketing
settings null, the gain it could add lies between them.

## Fixtures that moved

None: neither candidate was kept, so the suites are as the series head left
them.
