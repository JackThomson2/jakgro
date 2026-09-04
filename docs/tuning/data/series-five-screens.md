## Null probe shortened by the margin above beta (matched, rejected)
- vs head, depth 8 / 1 s x 7: A75 -2.70% nodes, +0.500 ply, NPS +0.5%; A0 -2.30% nodes, +0.200 ply; 4/10 positions identical; every gate passes; null contract holds.
- 4096 games at 50 ms, A75: -1.9 Elo [-8.2, +4.5], LLR -6.6, accept H0, no faults. The half ply did not convert. Patch kept as `series-five-patches/null-margin.patch`.
## Move ordering carried across searches of a game (matched, accepted)
- single searches tree-identical to the head at both profiles; every gate passes.
- 400-game replay at 50 ms: mean depth 6.561 vs 6.559 (+0.002 ply); tally +149 =125 -126.
- 4096 games at 50 ms, A75: +10.4 Elo [3.9, 16.8], LLR 4.9, accept H1, no faults.
## Aspiration window narrowed and re-centred on the fail (screened, not matched)
- radius 30 + follow the fail: A75 -9.04% nodes (more), -0.300 ply; A0 +3.86% fewer, +0.300 ply; 2 fixture rows.
- radius 40 + follow: A75 -5.82% (more), -0.300 ply.
- radius 50 + follow the fail alone: A75 -0.07%, 0.000 ply, 9/10 identical: inert.
- Rejected: every narrower window loses depth at the default profile. Patch kept as `series-five-patches/aspiration.patch`.
## Reduce more at cut nodes and where nothing improved (screened, not matched)
- both, vs head: A75 -1.28% nodes (more), 0.000 ply; A0 +3.75% fewer, +0.300 ply; one score-only fixture row; every gate passes.
- cut-node ply alone: A75 -6.25% (more), 0.000 ply. not-improving ply alone: A75 +1.20% fewer, 0.000 ply.
- Rejected: no form gains depth at the default profile. Patch kept as `series-five-patches/lmr-cut-improving.patch`.
## Follow-up history keyed by the side's own previous move (screened, not matched)
- agreement rule, vs head: A75 +0.38% fewer nodes, -0.300 ply; A0 +1.00% fewer, +0.100 ply; no fixture moved; every gate passes.
- Rejected: no depth at the default profile. Patch kept as `series-five-patches/followup-history.patch`.
## Evaluation batch throughput (tree-identical, informational)
- eight blocks at zero vs base, depth 8 / 1 s x 7: A75 -2.1% NPS, A0 -3.3% NPS; after six blocks -5.1% A75; 250 ms x 3 readings vary +-3% on identical code.
