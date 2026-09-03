## Move-count pruning at depths five to eight (screened, not matched)
- gate only, narrow improving: tree identical to parent — limits 43..102 at depths 5..8 never reached.
- limit 3+d^2 (x2 improving), narrow improving: A75 -3.84% nodes (more), -0.100 ply; A0 +7.38%, 0.000 ply; sacrifice gate pass; 2 personality rows.
- limit 3+d^2, wide improving (every interior node): A75 +10.49% fewer nodes, +0.100 ply; A0 +12.52%, -0.100 ply; sacrifice gate FAIL; 5 personality rows.
- with the original limit and wide improving: A75 -4.73% nodes, 0.000 ply; A0 +2.94%, 0.000 ply; sacrifice pass; 2 rows. Match started and stopped at 40 pairs on the screen.
## Quiet futility past depth two (screened, not matched)
- depth 4, margin 120+140d: A75 0.52% fewer nodes, 0.000 ply; A0 0.81%, 0.000 ply.
- depth 4, margin 120+100d: A75 -0.41% (more), -0.100 ply; A0 1.76%, 0.000 ply.
## Interior losing-capture pruning (screened, not matched)
- see < -20 d^2, depth <= 6: A75 -3.68% nodes (more), 0.000 ply; A0 +4.76%, +0.100 ply; sacrifice pass; 0 personality rows.
- see < -30 d, depth <= 6: A75 -5.57% (more), 0.000 ply; A0 +4.13%, 0.000 ply.
- see < -30 d, depth <= 3: A75 -7.83% (more), 0.000 ply; A0 +3.27%, -0.100 ply.
## Quiescence check filter (parked 43925a2, screened on the refit)
- filter alone: A75 3.87% fewer nodes, +0.100 ply; A0 3.56%, -0.200 ply; sacrifice pass; 1 personality row.
- filter + one more quiescence check per line at styled profiles: A75 -8.36% (more), +0.100 ply; sacrifice gate FAIL; 3 rows.
## Clock: soft limit scaled by best-move node share and stability (matched, rejected)
- 1.0+0.01, 2000 games: +2.95 Elo [-6.54, +12.45], LLR -0.87, no verdict.
- 1.0+0.01, 4096 games: -0.68 Elo [-7.62, +6.27], LLR -4.52, accept H0.
