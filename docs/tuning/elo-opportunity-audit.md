# Elo opportunity audit, October 2026

This report is the deep pass over everything that could still raise the
engine's playing strength. It was produced by thirteen read-only research
passes over the working tree (HEAD `e6e36ac` plus the uncommitted Syzygy
support, which the audit treats as the current engine), each comparing one
subsystem against Stockfish (the January 2023 `a08b8d4e` tree at
`artifacts/rating/src/Stockfish/src` and the 2026 "version 19" tree at
`artifacts/rating/stockfish/src`) and against the public record of other
engines, and by a set of fresh measurements on this host. Everything measured
here is labelled **EVIDENCE**; every estimate is labelled **GUESS** and given as
a range. Line numbers refer to the working tree as audited; function names are
the durable reference.

Two facts frame every item below:

- **The engine is tested at depth 7 and plays rated games at depth 17–20.**
  Self-play at 50 ms per move completes depth ~7 (styled) and `1.0+0.01`
  averages 27 ms per move; the external gauntlet reaches mean depth 17.2 at
  30+0.3 and 20.3 at 120+1 (`artifacts/research/round2-findings.md`). Several
  accepted and rejected results in `docs/tuning` were regime artifacts: the
  styled-root time cost is 45 Elo at 50 ms and 15 at 10+0.1; the forecast fix
  measured +15.6 at 50 ms and +3.6 at `1.0+0.01`. Nothing in the clock path,
  the table replacement policy, or null-move verification has ever been
  measured above 27 ms per move. Many items below are therefore "re-measure at
  10+0.1" rather than "new idea".
- **The default profile's root personality is inert at tournament depth.**
  Measured here (section 1.3): the styled root completes no verification and
  selects no move beyond depth 6–8, yet still spends 4.6% of the nodes at
  depth 12 and 1.5% at depth 16. Whatever style the engine shows at 10+0.1
  (checks ratio 0.71 against Aggression 0) comes from the interior couplings
  — the forcing-order bonus, the third check extension and the pruning
  exemptions — not from the root choice the fixtures gate.

## 1. Measurements taken for this audit

All runs used a release build of the working tree with debug symbols
(`CARGO_PROFILE_RELEASE_DEBUG=1`, `cargo +1.98.1 build --release --locked
--offline`), one thread, Aggression 75 unless stated, positions from
`docs/tuning/data/selective-search-confirmation.epd`. The build is
node-for-node identical to HEAD at depth 13 over the 40-position signature
(26,995,867 nodes at Aggression 75, 24,857,646 at Aggression 0), so the
uncommitted Syzygy support changes nothing without a `SyzygyPath`.

### 1.1 Cycle profile at the self-play hash size (Hash 16, 600k nodes × 40 positions, 2.25 Mnps)

| Symbol | Share |
|---|---:|
| `AccumulatorStack::evaluate_avx512` (NNUE) | 19.3% |
| `negamax` body | 15.4% |
| `MovePicker::next` | 11.9% |
| `MovePicker::generate_quiets` | 7.8% |
| `SearchContext::probe_table` | 6.5% |
| quiet sorts (`sort_unstable_by` on `QuietMove`: quicksort + merge + small sorts) | 6.3% |
| `MoveFacts::see` | 5.1% |
| `quiescence` body | 4.8% |
| `Board::play_unchecked_with_piece` | 3.9% |
| `move_gives_check` | 3.9% |
| `MovePicker::generate_tacticals` | 2.6% |
| `TranspositionTable::store_probed` | 1.7% |
| everything else (memmove, `legal_move_exists` 0.75%, `update_pv`, telemetry, …) | ~11% |

At Hash 64 the earlier profile put `probe_table` at 17.9% (87% of it a cache
miss on the first bucket load); at Hash 16 the table is L3-resident on this
host and the stall shrinks to 6.5%. Under an 88-game self-play load, or on a
rating-list machine with another engine running, the 64 MiB figure is the
representative one.

### 1.2 Search telemetry at depth 14 (40 positions, Hash 16)

| Counter | Aggression 0 | Aggression 75 |
|---|---:|---:|
| Nodes | 37.40M | 38.26M |
| Quiescence share of nodes | 62.0% | 61.8% |
| Interior TT probes: hit / cutoff | 47.5% / 8.1% | 47.5% / 7.8% |
| Quiescence TT probes: hit / cutoff | 12.0% / 11.0% | 11.6% / 10.6% |
| Static evaluations per node | 0.65 | 0.65 |
| Null move: attempts / fail-high rate / probe nodes | 726k / 50.2% / 8.5% of nodes | 748k / 51.6% / 9.2% |
| Null verifications (depth ≥ 12) | 1,122 | 1,342 |
| Reverse futility: cutoffs / attempts | 7.24M / 12.44M (58%) | 7.59M / 12.79M |
| Quiet futility pruned / move-count pruned | 9.74M / 23.48M | 9.72M / 22.57M |
| LMR: moves considered / reduced | 25.14M / 5.16M (20.5%) | 25.69M / 5.16M (20.1%) |
| LMR re-searches (reduced search beat alpha) | 20,617 = **0.40% of reductions** | 21,205 = 0.41% |
| Re-searches that then failed high | 51.7% | 52.9% |
| Aspiration: iterations / fail-lows / fail-highs / re-search nodes | 543 / 6 / 17 / 0.6% | 546 / 6 / 20 / 1.4% |
| Capture cutoffs: mean index of the cutting capture | 0.25 | 0.23 |
| Personality root nodes | 0 | 2.4% of nodes |

Three readings matter for the list. The late-move reductions almost never
need a re-search (0.4%), which is the signature of reductions far too mild
and too narrowly applied — Stockfish re-searches a double-digit percentage of
reduced moves. The ±50 aspiration window fails in 4% of iterations and costs
under 1.5% of nodes, which is why narrowing it looked useless at 50 ms and why
it should be re-screened at depth 16+. Reverse futility settles 58% of the
nodes where it is tried.

### 1.3 The styled root by depth (60 positions, Aggression 75, cumulative over iterations)

| Depth | Personality nodes | Attempts | Completed verifications | Budget exhaustions | Selections |
|---:|---:|---:|---:|---:|---:|
| 6 | 20.2% | 236 | 195 | 42 | 6 |
| 8 | 12.6% | 285 | 207 | 87 | 6 |
| 10 | 7.3% | 316 | 207 | 144 | 6 |
| 12 | 4.6% | 329 | 207 | 212 | 6 |
| 14 | 2.6% | 333 | 207 | 293 | 6 |
| 16 | 1.5% | 337 | 207 | 384 | 6 |

Counters are read from `info string personality …` with `debug on`
(`src/uci/session.rs`, `write_search_result`). No verification completes
after depth 8 and no styled selection happens after depth 6 in sixty
positions; every iteration from depth 9 on exhausts its node budget and
returns the objective move. At tournament depth the root personality is a
pure cost (1.5–4.6% of nodes) and the 20k–400k-node fixtures gate a regime the
engine never plays in.

### 1.4 Table size against nodes and time (depth 18, 16 positions, Aggression 75)

| Hash | Nodes | Time | NPS |
|---:|---:|---:|---:|
| 16 MiB | 85.52M | 38.9 s | 2.20M |
| 128 MiB | 91.75M | 47.2 s | 1.94M |
| 1024 MiB | 87.20M | 44.5 s | 1.96M |

At ~5M nodes per position (five times the 16 MiB entry count) a larger table
does not reduce the tree and costs 12–20% of time-to-depth. This replicates
the earlier depth-22 result and is explained by the replacement policy
(item T1): a saturated bucket refuses inexact stores that are shallower than
its current-generation occupants, so a small table degrades gracefully into a
depth-preferred one. Whether that holds across moves of a game, where the
previous search's tree is what a larger table would preserve, is the open
question the 10+0.1 match in section 1.8 addresses.

### 1.5 Lazy SMP scaling (Hash 256, `go movetime 3000`, six positions)

| Threads | Median NPS | Mean completed depth |
|---:|---:|---:|
| 1 | 1.91M | 18.0 |
| 2 | 3.88M | 18.8 |
| 4 | 7.84M | 20.0 |
| 8 | 15.76M | 20.0 |
| 16 | 31.58M | 22.0 |
| 32 | 59.39M | 22.0 |

Throughput scales linearly to 32 threads (31×). Depth at a fixed three
seconds gains about a ply per doubling to 4 threads, stalls at 8, and gains
two plies at 16; six positions make depth a weak proxy, but the 8-thread
plateau is consistent with the diversification policy saturating (item M3).

### 1.6 Time use in rated games (800 games at 120+1, `artifacts/rating/runs/ccrl-120s.pgn`)

| Move | Jakgro s/move | Opponents s/move | Jakgro depth | Opponents depth | Jakgro clock left | Opponents clock left |
|---:|---:|---:|---:|---:|---:|---:|
| 9 (first out of book) | 7.17 | 11.75 | 20.4 | 25.1 | 100% | 100% |
| 15 | 5.57 | 7.43 | 20.4 | 23.7 | 73% | 62% |
| 21 | 4.32 | 5.79 | 19.9 | 23.3 | 53% | 34% |
| 27 | 3.37 | 2.76 | 19.4 | 21.1 | 39% | 16% |
| 33 | 2.65 | 1.54 | 18.9 | 19.7 | 28% | 10% |
| 45 | 1.84 | 1.23 | 18.9 | 21.1 | 16% | 6% |
| 60 | 1.38 | 1.04 | 20.2 | 24.7 | 10% | 3% |
| 72 | 1.19 | 1.00 | 21.4 | 28.1 | 7% | 2% |

Jakgro's spend decays geometrically (`soft = remaining/15 + 0.75·inc`, of
which the forecast rule uses about 70%); the Stockfish-derived opponents
spend faster early and then live on the increment. Jakgro's depth falls from
20.4 to 18.7 through the middlegame, which is also where 85% of its losses
begin (median collapse move 24–25). None of this says which profile is right;
it says the profile has never been measured at these controls (section 5).

### 1.7 Late-move-reduction coverage screen (depth 13, 40 positions, node counts only)

| Variant | Aggression 75 | Aggression 0 |
|---|---:|---:|
| HEAD (`LMR_MIN_MOVE_INDEX 3`, `LMR_MIN_CHILD_DEPTH 3`, reduced depth floor 2) | 26,995,867 | 24,857,646 |
| A: reduce from move index 2 and child depth 2 | 27,136,559 (+0.5%) | 24,632,131 (−0.9%) |
| B: A + let the reduced search drop to depth 1 | 26,491,790 (−1.9%) | 22,116,926 (−11.0%) |
| C: B + also reduce checking and king-zone quiets | 23,831,363 (−11.7%) | 20,323,165 (−18.2%) |

Node counts are a screen, not Elo; the style fixtures and a 10+0.1 match
decide (item S1). The point is the size of the headroom: the exemption of
every checking and king-zone quiet from reduction — applied at Aggression 0
too — costs a tenth to a fifth of the tree at depth 13.

### 1.8 Hash 16 against Hash 256 at 10+0.1 (same binary, fastchess, `8moves_v3.pgn`, concurrency 88)

2048 games (1024 openings, colours reversed), 1250 draws (61%): Hash 16
scored 49.56%, i.e. **−3.1 ± 9.3 Elo** against Hash 256 (fastchess
normalised −4.9 ± 15.1; pentanomial 38/250/466/232/38). At ~300 ms per move
(~0.6M nodes at the contended throughput of 88 concurrent games) neither the
sixteen-fold capacity nor the ~10% single-thread throughput cost of the larger
table is measurable. Two readings: the self-play default of 16 MiB is not a
handicap at this control, and the realistic book draws 61% of games against
25% for the random-walk book, so every Elo figure is book-dependent and game
counts per Elo roughly double (item X2). The same experiment at 60+0.6 (2048
games, ~1.2 h) is the next step (T9).

## 2. Where the Elo is: the twelve biggest levers

Ranked by expected Elo per unit of effort, with the regime in which the gain
shows. Each entry is expanded in section 3.

1. **Move the acceptance gate to 10+0.1 with a real sequential SPRT, a realistic
   book, a node-based virtual clock and a `bench` signature (X1–X5).** Not
   Elo by itself; it decides whether the next hundred experiments measure
   tournament play or depth-7 play. At least four landed or parked results
   were regime artifacts.
2. **Regenerate the NNUE corpus with the current head as teacher and Syzygy on
   both sides, then retrain from scratch on a long schedule (D1, D4).** The
   corpus is 58% labelled by a teacher ~85 Elo weaker from 10k-node searches
   and a third of its "+2.5 at eight pieces" rows are draws. GUESS +10..+30 at
   fixed nodes, at least as much at long controls.
3. **Late-move reductions: coverage, floor, exemptions, cut-node and hash-capture
   terms, continuous history term, post-re-search depth adjustment (S1–S3).**
   0.4% re-search rate and 12–18% of the depth-13 tree in two exemptions.
   GUESS +5..+20, with style gates to re-pin.
4. **Correction history in its canonical form (Q1).** Four rejected forms were
   all cold per search, keyed by material counts, or applied only to pruning
   margins. GUESS +3..+12 at 10 s per move.
5. **Stop paying for a styled root that cannot finish (A1–A3).** Skip or
   rescale the styled pass once its budget cannot complete a verification,
   and make the forcing-order bonus a bounded history term. Elo at equal
   style; GUESS +5..+15 at 50 ms, +2..+8 at 10+0.1.
6. **Table replacement under saturation, keyed exclusion searches, a PV bit
   (T1, T3, T4).** The policy was written for 50 ms trees; at 4M nodes per move
   it refuses most quiescence stores and evicts deep stale entries first.
   GUESS +0..+8 at 30+0.3.
7. **Hide the table miss: prefetch from a predicted child key before the move is
   made, reuse `CheckMasks` for every emitted move, one-word quiet sort (P1–P4).**
   GUESS +8..+15% NPS under load, +4..+8% single-threaded; ≈ +6..+15 Elo.
8. **Null-move verification, ProbCut, singular extensions, IIR scaling,
   lmr-depth-keyed futility and the aspiration window at depth 16+ (S4–S9).**
   Each was accepted or rejected at depth 7, where the branch never fires or
   fires on the thinnest band of the tree.
9. **Time management above 27 ms per move (C1–C5).** Divisor, increment
   weight, hard cap, stability scaling and the partial-iteration result have
   never been measured where they matter; the 15-move divisor was chosen at
   `1.0+0.01`, where its neighbours were statistically indistinguishable.
10. **Endgame knowledge beyond five men (E1–E3).** 6-man tables, rule-50
    damping of the static evaluation, a mate-by-zeroing-move fix.
11. **SMP for users with cores (M1–M4).** Warm-start helper histories, Hash
    advice for multi-thread play, a persistent pool, thread voting; measure 8
    and 16 threads.
12. **Protocol robustness (U1–U4).** A negative clock drops the `go` and forfeits
    the game under cutechess time margins; `Hash` above 1024 is silently
    refused; the first search after `ucinewgame` re-zeroes the table; `Ponder`
    is not advertised so GUIs never ponder.

## 3. The list

Format of every item: what to change; the mechanism and where it lives; the
evidence; the Elo expectation (EVIDENCE or GUESS) at the short-control test
regime and at long controls; effort (S < 1 day, M 1–3 days, L > 3 days); risk,
including which fixtures would re-pin; the cheapest discriminating test; and
whether it was tried before. "Gate 0" is the fixed-depth node screen over the
40-position signature, "Gate 1" a 4096-game 50 ms match (3 min at concurrency
88), "Gate 2" 4096 games at 10+0.1 (24 min), "Gate 3" 4096 games at 30+0.3
(75 min). Style changes additionally report the Aggression 75/0 forcing and
check ratios with the pair bootstrap in `artifacts/research/tools/style_bootstrap.py`.

### 3.1 Search: reductions, pruning, extensions (S)

**S1. Widen late-move-reduction coverage and let reductions reach depth 1.**
`late_move_reduction` (`algorithm.rs`) reduces only from move index 3 at child
depth ≥ 3 (index 6 at depth 2), never a checking, castling or king-zone
quiet, never a killer, hash or PV move, never when in check, and never below a
remaining depth of 2; Stockfish reduces from the second move at depth ≥ 2 down
to depth 1 with no exemption for checks, killers or king-zone moves (its only
gate concerns captures at nodes that were once on the principal variation). Evidence: 0.4% of reduced moves need
a re-search (section 1.2); dropping the floor and the check/king-zone
exemptions removes 11.7% (Aggression 75) and 18.2% (Aggression 0) of the
depth-13 tree (section 1.7). Elo: GUESS +3..+12 at 50 ms, +5..+15 at 10+0.1,
larger if combined with S2. Effort S. Risk: the check and king-zone exemptions
are style-bearing at Aggression 75 — keep them as a reduction of one ply less
rather than a full exemption, measure the check ratio; every fixed-node
fixture re-pins. Test: Gate 0 (done above), Gate 1 with `measure_style.py
--check`, Gate 2. Prior: killers reduced instead of exempt −1.4, king-zone LMR
+1.4, LMR floor 1 +5.6/+1.4 (`search-table-clock-series.md`,
`search-store-razor-series.md`), all at 50 ms and each alone.

**S2. Add the cut-node, hash-capture and continuous-history reduction terms.**
Jakgro has no cut/all-node distinction (`negamax` carries only
`pv_node = beta − alpha > 1`), no `ttCapture` term, and a three-step history
term (±1 at ±5461). Stockfish 15 adds +2 at cut nodes (~3 Elo by its own
accounting), +1 when the hash move is a capture, −2 for `ttPv` nodes and
`statScore/12800` continuously (~30 Elo in aggregate, `search.cpp` 1129–1173
of the 2023 tree). Cut-node reductions measured −12 Elo here at 50 ms
(`search-selectivity-series.md`), and the series-five screen is
profile-dependent: one more ply at cut nodes searched 6.25% more nodes at
Aggression 75 but 3.75% fewer and +0.3 ply at Aggression 0
(`docs/tuning/data/series-five-screens.md`) — the forcing-order bonus pushes
good quiets to later indices, so at the default profile deeper late reductions
hit the wrong moves. Screen cut-node terms at Aggression 0 and after O3. Elo: GUESS 0..+8 after S1. Effort S
each. Risk: fixture re-pins. Test: Gate 0 at depth 16 (not 13), Gate 2.

**S3. Post-reduction depth adjustment ("do deeper / do shallower").**
After a reduced search beats alpha the re-search runs at the full child depth
(`reduced_search_needs_research`); Stockfish re-searches one ply deeper when
the reduced score beat alpha by a margin (`value > alpha + 66 + 11·r` in the
2023 tree, `best + 53` in the 2026 tree) and one ply shallower when it barely
did. Elo: GUESS +1..+4 at both regimes. Effort S. Not tried. Test: Gate 0,
Gate 1, Gate 2 as a rider on S1.

**S4. Null-move verification policy at depth ≥ 12.**
`verified_null_move_cutoff` verifies from `NULL_VERIFICATION_MIN_DEPTH = 12`
with a `depth − R` search in `SearchMode::Verification`, which forbids null
moves in the whole subtree for both sides. Stockfish verifies from depth 14
(2023) / 16 (2026) and disables null only for the side to move and only for
`3·(depth−R)/4` plies; Stash likewise. At depth 20, R is 9–12, so each
verification is a null-free depth-8 to depth-11 search. Evidence: the branch
never fired in any 50 ms or `1.0+0.01` match (depth ~7); section 1.2 shows
1,122 verifications at depth 14 already. Elo: 50 ms inert (EVIDENCE); GUESS
0..+6 at 10+0.1 and 30+0.3. Variants: threshold 12→16, partial disable (mover
only, `3(d−R)/4` plies), off. Effort S. Risk: zugzwang in minor-piece endings
(the material rule still blocks pawn-only and single-minor endings);
`tests/null_move_regression.rs` tolerances. Test: `depth_driver.py … depth 16`
telemetry (`null_verification_nodes`), Gate 2. Prior: only the null margin
shortening (−1.9) was screened.

**S5. Quiet futility keyed to the move's reduced depth, not the node depth.**
`should_prune_quiet_move` uses `depth ≤ 6` and `eval + 100 + 100·depth ≤ alpha`
with a `history > 0` exemption; Stockfish uses `lmrDepth` (depth after the
move's own reduction) with cap 12–13 and `103 + 136·lmrDepth + history/53`
(~13 Elo in its accounting, and marked in the 2026 tree as "scales well").
Keying to the node depth is why raising the caps to 9/8 changed nothing
(−2.3) and the round-two cap sweep found 0% node change: margins of 900 cp at
depth 8 never trigger. Requires computing the reduction before the pruning
decision (move the `late_move_reduction` call ahead of
`should_prune_quiet_move`). Elo: GUESS −3..+3 at 50 ms, 0..+8 at 10+0.1.
Effort S–M. Risk: sacrifice fixtures (keep the pawn-push, check and king-zone
exemptions first); re-pins. Test: Gate 0 at depth 13 and 16 (expect −5..−15%
nodes), Gate 1 with style check, Gate 2.

**S6. Singular extension family at long control.**
`SINGULAR_MIN_DEPTH 4`, margin `2·depth`, extension +1, multi-cut; no double
extension, no negative extension when the hash move's score is above beta, no
`ttPv` term. Stockfish 15 extends by 2 when the exclusion search fails low by
a further 25 cp at non-PV nodes (capped per line), reduces by 1–2 when the
hash score is ≥ beta; the 2026 tree notes that lower extension margins and a
higher singular beta "scale well". Prior: double/negative measured −0.5
[−6.7, +5.7] at 50 ms where the depth ≥ 4 band is a few hundred nodes.
Elo: GUESS 0..+6 at 10+0.1. Effort S. Risk: explosion without a per-line
double-extension cap. Test: Gate 2 directly (node counts rise by design).

**S7. ProbCut from depth 5 at non-PV nodes.**
Rejected −3.2 [−9.4, +3.0] at 50 ms "from depth five, margin 200, four plies
shallower" — at depth 7 the depth ≥ 5 nodes are the thinnest band of the
tree. Stockfish attributes ~10 Elo to it and runs it from depth 3 in 2026.
Form: `pcBeta = beta + 200 − 50·improving`, skip when the table already bounds
the node below `pcBeta` at depth ≥ d−3, good captures only (SEE already
computed in the picker), quiescence then a `depth − 4` null window. Elo: GUESS
−3..+2 at 50 ms, 0..+6 at 10+0.1. Effort S–M. Test: Gate 0 at depth 16, Gate 2.

**S8. Internal iterative reduction scaling.**
IIR reduces every node type from depth 4 when the table holds no move,
including PV and all-nodes; Stockfish never reduces all-nodes, reduces PV
nodes by 3 and cut nodes by 2 from depth 9 (2023), and the 2026 tree warns
that more aggressive IIR "scales poorly". At 120+1 the table misses far more
often than at 50 ms (4M nodes per move against 1M entries at Hash 16), so IIR
fires more often exactly where it costs most. Variants: `IIR_MIN_DEPTH` 4→6;
no IIR at PV nodes; cut-node only once S2 exists. Elo: GUESS −3..0 at 50 ms
(IIR was +12 there), 0..+5 at 10+0.1. Effort S. Test: Gate 0 at depth 16 at
Hash 16 and 64, Gate 2 at Hash 64.

**S9. Aspiration window radius and widening at depth 16+.**
`ASPIRATION_INITIAL 50`, radius doubled on failure and re-centred on the
previous iteration's score; Stockfish starts at ~10–16 cp (2023) or
`5 + |avg|²/10193` (2026), widens ×1.25–1.37, and reduces the depth on
repeated fail-highs. Narrower windows lost at 50 ms twice (−10 Elo at 16 and 30 cp;
−0.3 ply at radius 30/40 at Aggression 75) where depth-7 scores swing most —
but the same radius-30 screen gained 3.9% nodes and +0.3 ply at Aggression 0
(`series-five-screens.md`), because every aspiration retry re-runs the styled
root pass (A1, A4). Section 1.2: the window
fails in 4% of iterations and costs 0.6–1.4% of nodes, so at depth 14 it is
doing almost nothing. Elo: GUESS −5..0 at 50 ms, 0..+4 at 10+0.1. Effort S.
Test: `depth_driver.py` at depth 16 with radius 50/35/25 reading
`aspiration_research_nodes`, then Gate 2.

**S10. Check extension shape.**
Every in-check node is extended by a ply while `extensions_used <
2 + aggression/50` (3 at the default) at every depth; Stockfish 15 extends
checks only at depth > 9 with a non-trivial eval and the 2026 tree has no
check extension at all. Capping at 2 read +5.5 and −0.8 over 8192 games.
Gate the third extension on the checking move being SEE-safe (the roadmap's
own next step) and consider dropping extensions at depth ≤ 3 where the check
budget is spent on noise. Elo: GUESS +1..+4 at 50 ms, 0..+6 at 10+0.1; the
check ratio will move. Effort S–M. Risk: style gates. Test: Gate 1 with the
style bootstrap, Gate 2.

**S11. Move-count pruning that actually fires.**
`late_move_pruning_limit = 6 + 3·depth²/2` (doubled when improving) is 19 at
depth 3 and 102 at depth 8, counting every emitted move; Stockfish prunes at
`(3 + depth²)/(2 − improving)` with no depth cap (33 at depth 8 not
improving). The Stockfish formula searched more nodes here (series-four
screens) — a move-ordering limit: without O1/O4 the late quiets include good
moves. Re-screen after O1–O4 land; until then leave it. Elo: GUESS 0..+5 after
ordering work. Effort S.

**S12. Key singular exclusion searches in the table.**
The exclusion search probes the parent's key but may neither cut nor store, so
every iteration re-expands it; the second strength series measured exclusion
searches at 10.7–30.6% of nodes. Stockfish keys them `key ^ make_key(excluded)`
and stores normally. Implementation: mix `encode_move(excluded)` into the key
passed to `probe_table`/`store_probed` when `excluded.is_some()` and allow the
exit store. Elo: GUESS +1..+5 at 50 ms, more at depth 17–20 where the probe
depth `(depth−1)/2` is 8–9. Effort S. Test: Gate 0 (expect fewer nodes),
Gate 1, Gate 2.

**S13. An SPSA pass over the never-tuned search constants at 10+0.1.**
The single SPSA run (24 constants, 50 ms) read −0.3 at its own regime and
+4.6 clocked. Never tuned: `LMR_MIN_MOVE_INDEX`, `LMR_SHALLOW_*`,
`LMR_MIN_CHILD_DEPTH`, the LMR floor, `IIR_MIN_DEPTH`, `SINGULAR_MIN_DEPTH`,
the singular table slack and probe depth, `NULL_MOVE_MIN_DEPTH`,
`NULL_VERIFICATION_MIN_DEPTH`, the null excess cap and material rule,
`RAZOR_*` (the README calls them untuned), the check budget, `QUIESCENCE_DEPTH`,
the aspiration widening factor, `FIRST_CAPTURE_HISTORY_BONUS_DIVISOR`,
`HISTORY_MAX`, the PV/not-improving LMR terms. Requires X5 (tunable layer and
driver). Elo: GUESS +3..+10 at 10+0.1. Effort M–L including X5.

**S14. Improving signal when the evaluation is missing.**
`record_static_evaluation` returns `true` (improving) when the node is in
check, when ply < 2 and when the grandparent was in check; for reverse
futility "improving" narrows the margin by 40, so the conservative default
prunes more. Stockfish chains the evaluation through checks (`ss−2` eval
copied) and treats the missing case as not improving. Elo: GUESS ±2. Effort S.
Rider on another Gate 1 run.

### 3.2 Quiescence and the use of the static evaluation (Q)

**Q1. Correction history, canonical form, persisted across the game.**
No correction history exists. The four rejected forms differ from the
canonical one in ways that explain the results: the table was created fresh
per search (cold every move at 50 ms; the public gains grow with time
control), keys were pawn bitboards plus piece counts rather than per-colour
non-pawn placement, minor and major keys and a continuation key, captures
were allowed as the updating best move (fatal for a tactical engine, whose
best moves are often captures), the learning rate was ~8× below the public
form, and the two NNUE-era tests never touched the stand-pat. The specific
mechanism here: in-tree repetitions return 0 while the network reads +2.5 in
drawn endings (round two: 94.5% of ≥ +2.5 evaluations at ≤ 5 men are
tablebase draws, 66–68% judged equal by an oracle at ≤ 8 men); a per-colour
non-pawn key is the only in-search device that learns "this material class
returns far less than its evaluation" above five men, where tablebases stop.
Spec: carry the table in `SearchMemory` and halve it like `MoveOrdering::age`;
apply `raw + correction` to the pruning evaluation, `improving`, RFP, null,
quiet futility, the razoring hand-off and the quiescence stand-pat; store the
raw evaluation in the table; update after the move loop only in
`SearchMode::Normal`, not in check, not excluded, when the best move is not a
capture and the bound is consistent, with `clamp((best − corrected)·depth/8,
±256)` and the gravity update. Elo: GUESS −3..+6 at 50 ms, +3..+12 at 10 s per
move (public engines report two-digit gains). Effort M. Risk: root-guard
sign flips re-pin 2–4 fixtures; ~1% NPS. Test: Gate 1 for sanity, **Gate 2
decisive**, Gate 3; control = same binary with the weights zeroed.

**Q2. Damp the static evaluation toward zero with the halfmove clock.**
Nothing scales the evaluation by the fifty-move clock; the network has no
clock input, so the rule is unlearnable by construction, and Stockfish keeps
`v −= v·rule50/199` with NNUE for that reason. Round two: 93% of the 185
result-worsening moves in 1800 gauntlet games were at clock ≥ 50 (median 92),
17 Elo of unconverted wins; 5-man tables now cover most of those, so the
residual is 6+ men and table-less hosts. Must be applied after reading the
cached evaluation (the table caches the raw stand-pat across clock classes
below 80: `store_quiescence_result`, `Entry::static_evaluation`,
`clock_class`), never inside `static_score`. Elo: GUESS 0..+2 at 50 ms,
+1..+4 at 30+0.3 without tables. Effort S. Risk: trees change wherever the
clock is non-zero; a knife-edge sacrifice fixture flipped when this was tried
pre-NNUE (+6 within noise, `nnue-aggression75.md`). Test: the syzygy report's
"keeping a won position" and "converting" harnesses with tables off, then
Gate 2 and a 1000-game gauntlet audited with `tb_probe.py`.

**Q3. Prune quiet evasions in quiescence once a non-losing evasion exists.**
In check, every legal evasion is searched (`MovePickerMode::Quiescence
{in_check: true}`); Stockfish stops searching quiet evasions and SEE-losing
captures once `bestValue` is not a loss. In-check quiescence nodes come from
the first-ply quiet checks at every profile and from depth-0 checks once the
extensions are spent; the quiet-check ablation read +5.9 at 50 ms, so their
refutation cost is real, and this keeps the checks (style) while cheapening
the defence. Elo: GUESS 0..+6 at 50 ms, 0..+4 at 10+0.1. Effort S. Risk:
check ratios may rise slightly. Test: Gate 0 (saving), Gate 1, Gate 2.

**Q4. Move-count and SEE-gap pruning in quiescence.**
Only `SEE < 0` (computed only for lower-takes-higher captures) and the
`stand_pat + value(captured) + 120 ≤ alpha` delta rule exist; Stockfish also
prunes non-check, non-recapture captures after the second one and any capture
whose SEE cannot lift `stand_pat + margin` to alpha. Proposal: `index ≥ 3`
prune for unexempted captures; `stand_pat + 120 + see ≤ alpha` where SEE is
known. Elo: GUESS 0..+5. Effort S. Risk: king-zone captures are exempt at
aggression > 0 for style; re-pins. Do not hand-retune the delta margin (SPSA
moved 120→136 within noise).

**Q5. Quiet checks in quiescence at Aggression 0.**
`quiescence_check_budget` is 1 at every profile below 80, so this is an
objective-search question, not a personality one: two 50 ms screens disagree
(off: +5.9 [−0.6, +12.4] and −2.0 [−8.2, +4.3]); "horizon-only below 80"
screened +11.9 [−0.0, +23.8] in 1024 games and stalled on an 8 cp fixture
loss. Candidates: horizon ply only; SEE-safe checks only
(`static_exchange_eval_settle(board, move, attacker, 0)` is cheap). Elo: GUESS
+2..+8 at 50 ms, unknown at 10+0.1; the check ratio moves. Effort S–M. Risk:
moves Aggression-0 fixtures too (highest re-pin tax). Test: Gate 1 + style,
Gate 2.

**Q6. Quiescence table cutoff at PV-window nodes.**
`quiescence_entry_is_usable` cuts at every node including those with an open
window; Stockfish's quiescence cuts only at non-PV nodes, and the +12 Elo
"search past the hit" patch covered interior nodes only. Elo: GUESS ±2.
Effort S. Rider on T-series Gate 1.

**Q7. Draw-score randomisation.**
All three draw sources return exactly 0; Stockfish returns `1 − (nodes & 2)`
to break three-fold blindness. Elo: GUESS 0..+3 at 50 ms, ~0 at long control.
Effort S; all fixed-node fixtures change (still deterministic). Bundle with
Q2's re-pin.

**Q8. Incremental repetition detection.**
`repetition_draw` scans back up to `halfmove_clock/2` keys at every node;
Stockfish keeps an incremental `st->repetition`. GUESS +0.5..1.5% NPS,
tree-identical. Effort S.

**Q9. Why does a stored bound read as the evaluation lose here?**
Three forms lost 19–30 Elo while the same device gains in Stockfish and
Stash. Hypothesis worth one telemetry run: fail-soft fail-lows after razoring
and quiescence depth-0 `Lower = stand_pat` entries put scores far from the
evaluation into the table; a histogram of `tt_score − static_eval` by bound at
depth 13 would show it, and would also inform Q1's consistency guard. Not a
code change; a measurement.

### 3.3 Move ordering and history (O)

**O1. Symmetric statistics updates.**
On a quiet cutoff only the failed quiets are penalised (`record_quiet_cutoff`
with `failed_quiets()`); failed captures are consulted only on a capture
cutoff, although "good capture failed, quiet cut" is the dominant pattern
because good captures are searched first. Quiets get nothing on a capture
cutoff; there is no best-move bonus at exact PV nodes, no bonus for the quiet
hash move on a table cutoff (Stockfish ~2–3 Elo), and no bonus for the
opponent's previous quiet when a node fails low. Elo: GUESS 0..+5. Effort S.
Risk: re-pins. Test: Gate 0 over 150 positions, Gate 1, Gate 2.

**O2. History step size and saturation are regime-dependent.**
`history_bonus = min(64·depth², 8192)` saturates at depth 12 — never in the
50 ms regime, at every node in the top eight plies of a tournament tree — and
8192 is half of `HISTORY_MAX`, so one deep cutoff overwrites half an entry and
flips the `> 0` pruning exemptions and the ±5461 LMR step on single events.
Stockfish's maximum step is ~21% of its scale; Stash's 28%. Candidates: cap at
D/4 and D/8; a decaying malus along the failed-quiet list (Stockfish 921/1024
per earlier move). Elo: GUESS −2..+3 at 50 ms, 0..+6 at 10+0.1. Effort S.
Test: depth-15 node screen; Gate 2 decides (50 ms cannot see the saturated
regime). Prior: the 50 ms SPSA moved the cap 8192→8297.

**O3. Forcing-order bonus as a bounded, SEE-gated term.**
`forcing_order_bonus` puts every quiet check (`800_000 + 3000·aggression`) and
every attacking pawn push above every history-ordered quiet at every interior
node, including checks that hang the piece, and inflates the move index (and
so the reduction) of the best history moves. Removing it read +5.9 [−0.5,
+12.3] at 50 ms with forcing 0.994 and checks 0.980 — the only personality
knob whose removal left the style intact — and was never run at 10+0.1.
Stockfish grants its check bonus only when `see_ge(m, −75)`. Replace with
`history + HISTORY_MAX/2` for SEE-safe checks and `+HISTORY_MAX/4` for pushes,
or keep the tier only at child depth ≤ 3. Elo: GUESS +2..+6 at 50 ms, +2..+8
at 10+0.1; checks 0.98–1.00. Effort S. Risk: `bm75`/`bm100` fixtures. Test:
the existing switchboard binary (`artifacts/research/ablation-switchboard.patch`,
bit 8) at 10+0.1 first, then the gated variant at Gate 1 with the style
bootstrap (require checks lower CI ≥ 0.95).

**O4. Continuation history in a measurable form, then a second ply.**
The continuation table is keyed `(prev.piece, prev.to, cur.piece, cur.to)`
without colour (white's and black's statistics alias), weighted 1/8 and
sign-gated so it can never flip the butterfly sign — effectively a
tie-breaker. The "full weight +1.3" test bundled a follow-up table in one
patch; the follow-up screen kept the gate. Proposal: colour-index (or two
tables), i16 storage, weight sweep {1/4, 1/2, 1} without the sign gate,
screen by depth-13 nodes, match the best; only then add the 2-ply table with
Stockfish's weight ratio (390/520). Elo: GUESS 0..+6 at 50 ms, +3..+10 at
10+0.1. Effort M. Risk: re-pins; −1..−2% NPS from table traffic.

**O5. Ordering telemetry (prerequisite, zero Elo).**
Only captures record their cutoff index. Add interior cut-node count, cutoff
index histogram (0/1/2/3+), cutoffs by picker stage, quiet-stage entries and a
"first searched move was the hash move" counter, printable via an env-gated
`info string`; without them O1–O4 and S11 are tuned blind. Effort S.

**O6. Threat-aware quiet ordering.**
Stockfish's `threatByLesser` term (moving a piece out of a cheaper piece's
attack, or into one) needs four opponent attack bitboards per node, which
cozy-chess does not cache. Elo: GUESS 0..+5; NPS −1..−2%. Effort M. Test: Gate
0, Gate 1.

**O7. Bad captures before bad quiets.**
Bad captures are emitted after every quiet, including quiets with history
−16384; Stockfish emits them before quiets whose history is below a depth-
scaled threshold. Elo: GUESS 0..+3. Effort S. Rider.

**O8. Root ordering by subtree node counts.**
Not tried; ordering by previous scores lost −7.1 [−13.5, −0.8] with no
mechanism identified (styled-root or aspiration interaction?). Low priority
until O5 shows root fail-high waste. GUESS −2..+3.

### 3.4 Transposition table (T)

**T1. Replacement policy under saturation.**
`placement` drops an inexact store when the bucket is full and the victim is a
current-generation entry that is exact or deeper, and `replacement_index`
evicts any stale entry before any current one regardless of depth; cutoffs do
not refresh the entry's generation. At 50 ms (100–300k nodes per move)
buckets rarely fill; at 120+1 (~4.3M nodes per move against 1.05M entries at
Hash 16 or 4.2M at Hash 64) nearly every node attempts a store, so most
quiescence and leaf stores are refused and a depth-18 exact entry from the
previous move's PV loses to a current depth-0 quiescence entry (Stockfish:
`depth − 8·age`, always written on a miss, refreshed on every hit). Section 1.4
shows the policy is not harmful within one search; the cost, if any, is
across moves. Elo: GUESS 0..+8 at 30+0.3 and Hash 16–64, possibly negative.
Effort S. Variants: (i) Stockfish value `depth − 8·age` (+2 for exact),
always write on a miss; (ii) + refresh on cutoff; (iii) current policy but
never drop. Test: add a refused-store counter, `go depth 20` telemetry at Hash
16 vs 256 (expect the quiescence hit rate to collapse under the current
policy), then Gate 2 and Gate 3 at Hash 16, plus one Gate 3 at Hash 256 to
confirm it is not negative unsaturated. Prior: the four-ply same-position rule
alone measured +1.1/−1.5 at 50 ms.

**T2. Prefetch before make from a predicted child key.** See P1.

**T3. A PV bit in the free move bit.**
`encode_move` uses 15 bits, so bit 15 of the payload is free. Stockfish sets
`ttPv = PvNode || tte->is_pv()` and uses it to skip futility pruning, reduce
less (−2 plies), gate capture reductions, widen the singular margin,
propagate on fail-low and favour replacement. Jakgro has only the
window-based `pv_node` and the move-level `protected` flag, so a node that
was on the principal variation in an earlier iteration and is re-searched
with a null window gets full RFP and LMR. Elo: GUESS +2..+8 at 50 ms, as much
or more at long control. Effort M. Risk: every personality fixture re-pins.
Test: Gate 0, Gate 1, Gate 2.

**T4. Key singular exclusion searches.** See S12.

**T5. Index by the clock-mixed key.**
`index` uses the raw key, so the 21 singleton clock classes (80–100) of one
position compete for one 4-slot bucket and thrash in shuffle endings;
indexing by `mixed` spreads them and is tree-identical below clock 80. Elo:
GUESS 0..+2 in long shuffles. Effort S; needs the child's clock at prefetch
time. Test: node identity on normal positions plus node counts on clock-85+
FENs.

**T6. Cheap Stockfish table rules.**
(a) Cut off on an exact entry at `entry.depth() + 1 == depth`; (b) history
bonus/malus for a quiet hash move on a lower/upper-bound cutoff. GUESS 0..+3
each at 50 ms. Effort S. One patch at Gate 1; drop below +3.

**T7. Hash cap, clamp, hashfull and clear cost.**
`MAX_HASH_MIB 1024` is low for 16–32-thread users on a 188 GB host; `index` is
a 128-bit multiply with no power-of-two assumption, so raising the cap is one
constant (plus `selfplay.rs`). `Hash` is the one spin option that is rejected
rather than clamped, and the rejection is printed only in debug mode (U2).
`discard_entries` is a serial pass (~0.1–0.2 s per GiB per `ucinewgame`);
re-zero with `madvise(MADV_DONTNEED)` or in parallel before raising the cap.
`info hashfull` is absent (cosmetic, helps testers size the table). Elo ≈ 0
at CCRL sizes; GUESS 0..+5 for TCEC-like users. Effort S.

**T8. Smaller entries (10 or 12 bytes).**
+25–50% capacity at the price of the 64-bit torn-read proof and more
unpacking (the 24→16-byte compaction measured 1.4% slower). Only worth
building if T1's saturation experiment shows capacity matters at rated
controls, and CCRL runs 128–256 MiB anyway. Effort M. Not before T1.

**T9. The saturation experiment itself.**
Nodes-to-depth is not evidence of capacity (per-position ratios vary ±50%
and the drop rule makes a saturated table keep deeper entries). Cleaner
tests, same binary: time-to-depth-20 at Hash 16/64/256 interleaved;
fastchess `option.Hash=16` vs `option.Hash=256` at 60+0.6, 2048 games (~1.2 h,
±8.5); the same at 4 threads and 30+0.3. Section 1.8 is the first of these.

### 3.5 Time management and the iterative-deepening driver (C)

**C1. The distribution of time over a game has never been measured above 27 ms per move.**
`soft = usable/15 + 0.75·inc`, `hard = min(3·soft, usable)`, and the forecast
rule spends about 70% of soft on average. `DEFAULT_MOVES_TO_GO` 30→15 was
chosen at `1.0+0.01` (+18.8) where 20 (+13.8) and 12 (+12.2) were inside each
other's intervals and where the 5 ms margin, 10 ms overhead and 5–50 ms
reserve are 20–50% of the budget; the increment weight, the hard multiplier,
the reserve and the volatility hold were never A/B'd at all. Section 1.6 shows
the resulting profile against Stockfish-derived opponents. Arms for one
Gate 2 round (24 min each): (a) divisor 22, increment share 1; (b) divisor
30, increment share 1; (c) a Stockfish-shaped ply ramp `usable·min(0.012 +
(ply+3)^0.45·0.0039, 0.2)·k` with k matching (b)'s first-move spend; winner to
60+0.6, 2048 games. Elo: inert at 50 ms (EVIDENCE: fixed budgets); GUESS
+2..+8 at 10+0.1 and 60+0.6; likely negative at `1.0+0.01` (document the
inversion). Effort S. Risk: none to fixtures.

**C2. Continuous stability scaling of the soft budget.**
The only adaptivity is one `Extend` when the best move or score moved in the
last two iterations (a +50 jump counts like a −50 drop); nothing saves time
when the move has been stable for ten iterations, which at depth 17–20 is
common. Stockfish scales 0.5–0.7× when stable and up to ~2.2× on
within-iteration best-move changes or a falling evaluation (one-sided). The
rejected version (−0.7 at `1.0+0.01`) was measured at depth ~7, where no
long-stable signal exists and its effort term added time for contested moves
(the opposite sign to Stockfish's). Form: `soft_eff = soft · instability ·
falling`, instability from within-iteration changes (the signal exists at the
root loop), reduce to 0.8 after ≥ 6 stable iterations; keep the forecast
against `soft_eff`. Elo: GUESS +2..+8 at 10+0.1 and 60+0.6. Effort S–M. Test
jointly with or after C1 at Gate 2; report mean time per move so a pure
"spends more" effect is distinguishable.

**C3. Keep the partial iteration at a hard abort; relax the forecast toward a continuous rule.**
An aborted iteration is discarded even when its first move completed and a
later move beat it — exactly the case the extension was bought for.
Stockfish plays the partial best. "Best move of an abandoned iteration" alone
measured +1.7 [−1.5, +4.9] over 12,288 games, but the forecast made aborts
rare; a continuous rule (continue while `elapsed < c·soft`, abort at hard,
keep the partial result when ≥ 1 root move finished) makes aborts routine and
also removes the forecast's blind spot after a fail-low iteration. This is
also the only mechanism that can use the unspent third of a `movetime`
budget. Elo: GUESS 0..+5 at 10+0.1 and 60+0.6, 0..+4 at 50 ms. Effort M. Do
after C1/C2 (changes the average spend).

**C4. Safety caps: `hard ≤ 0.8·remaining`; `movestogo ≤ 2` must not spend to `usable`.**
At `movestogo 1` soft = hard = usable (remaining − 10 ms − ≤ 100 ms); at
`movestogo 2` an extension can drain the penultimate move and leave the last
one a zero budget (a depth-1 move or a forfeit under GUI jitter). Repeating
controls such as CCRL 40/15 send `movestogo`; no match here ever has. Elo ≈ 0
in increment play (EVIDENCE: no forfeits in 1800 games); prevents rare
forfeits in rated repeating controls. Effort S. Test: unit tests plus a
512-game fastchess run at `tc=40/60+0`.

**C5. Increment weight and hard multiplier.**
3/4 of the increment is spent per move against Stockfish's ~1–2× and Stash's
1/1.1; the hard limit at 3× soft (≈4.3× actual spend) binds only via the
extension. Fold the increment into C1; sweep the hard multiplier (2, 3) only
once C3 makes aborts routine. GUESS 0..+3.

**C6. `go movetime` uses ~68% of the budget — a test-regime fact.**
With an iteration growth ratio of 1.7–2 the forecast admits the next
iteration only while elapsed ≤ ~0.5·T, so fixed-time searches end anywhere in
[T/2, T] (34 of 50 ms measured). Under a clock the remainder is carried over;
at fixed time it is lost equally for both sides, so the 50 ms regime is really
a 34 ms regime and any change to the iteration growth ratio changes how much
of the 50 ms is used. Only C3 recovers it; no action for rated play.

**C7. Ponder budget.**
`go ponder` runs without a deadline and `ponderhit` arms the full budget from
that moment, so pondered time is a free bonus; Stockfish instead adds 25% to
the optimum when pondering is on. Correct and conservative; but see U4 —
GUIs never ponder because the option is not advertised.

### 3.6 NNUE architecture and inference (N)

The network: 768 piece-square inputs × 8 king buckets on a file-mirrored half
board (6144 features; both kings are features), one shared 512-unit feature
transformer per perspective in int16, squared clipped ReLU (0..255), a single
linear output layer per piece-count bucket (8 buckets, `(pieces−1)/4`), no
hidden layers; 3.55M trainable floats, 6.3 MB exported. Inference is 16–19% of
search time; lazy evaluation (skip in check, reuse the table's evaluation,
store early-exit evaluations) is already complete, the refresh cache is gated
to ≥ 17 pieces by measurement, and the x86 kernels saturate a compile-time
accumulation bound at exactly 512 units (`lanes_within_chunk` in
`kernels.rs`), so **any wider transformer first needs that bound widened**
(spill to i64 every 64 units, or `OUTPUT_WEIGHT_LIMIT` 63).

**N1. Keep training the shipped recipe.**
The shipped network's dev loss fell every epoch and the run stopped at its
epoch budget (8), not at convergence; the eight unused head-engine corpus
groups exist. Elo: GUESS +3..+10 at 50k nodes for 16–24 more epochs (prior
continuations: +10.7, +16.5, +15.2, +25.9 EVIDENCE). Effort: zero code, ~2.1 h
(`train --init-network nets/jakgro.nnue --epochs 16 --rate 0.0001 --rate-decay
0.9 --ema 0.9998 --factorize true --qat true --lambda 0.25` on d5). Risk:
every new net re-pins the 20k-node knife-edge fixtures. Test: 4096 games at
50k nodes, then 50 ms. Superseded by D1/D4 once a fresh corpus exists.

**N2. Trainer throughput: partition by hidden-unit slice, not by row shard.**
`SHARDS = 16` fixed worker threads (`--threads` affects only parsing and
metrics) each hold a dense 12.6 MB gradient zeroed per step; phase 3 sums all
sixteen per parameter (≈ 5.5 TB of reads per epoch), which is the 191 s
"adam" phase of each 476 s epoch — about 7× off the memory floor and on 16 of
96 vCPUs. A slice-partitioned design removes the reduction, scales to 48–96
workers and stays deterministic. Elo: none directly; GUESS 3–8× cheaper epochs
(from-scratch 80 epochs: 10.6 h → 2–4 h), which multiplies D1–D5 and makes
N3–N5 affordable. Effort M. Risk: float reassociation changes results; verify
by retraining the shipped continuation to the same dev MSE.

**N3. A hidden layer stack with pairwise multiplication.**
`(6144→512)×2 → pairwise clipped product (256 per perspective, u8) → 16 → 32
→ 1` per output bucket, Stockfish/bullet-style; the single linear output layer
cannot model interactions between units. Public: Viridithas 14 credits its
multilayer net for a large jump; Alexandria 9 adopted L2 16 / L3 32 with dual
activation; Stockfish has used stacks since 2020. Elo: GUESS +10..+30 at 50k
nodes at this data scale (the head adds ~135k parameters and is not
data-hungry); speed cost GUESS 3–6% NPS dense (−5..−10 Elo at 50 ms, less at
long control), ~1–2% with a sparse-input kernel. Effort L (format v4, three
kernel ISAs, trainer, loader proofs, fixture re-pin). Training ≈ 620 s/epoch
now, ≈ 3 h for 80 epochs after N2. Test: same-recipe 512 control vs L2/L3
candidate at 50k nodes and at 50 ms.

**N4. Widen the transformer to 768 or 1024.**
The only clean width test here: 128 vs 512 ≈ +13.7 Elo at fixed nodes, and the
wider net profited more from more training (+18.7 vs +6.9 for 80 vs 30
epochs). At 16–19% evaluation share, 1024 adds ≈ 11% search time (GUESS) → a
−15..−20 Elo tax at 50 ms by the measured slope (115·log₂ speedup), small at
10 s per move. Elo: GUESS +5..+15 at 50k nodes after full training, −5..+8 at
50 ms, positive at long control only if quality beats the tax. Effort S–M
(constant, kernel bound fix, trainer import; measure NPS with a random 1024
net first). Training ≈ 950 s/epoch (≈ 4 h for 80 epochs after N2).

**N5. 16 king buckets, finer where the data is.**
74.5% of rows have the king on ranks 1–2 in two buckets; a bullet-style
layout (rank 1 four buckets by file, rank 2 two, ranks 3–4 one each, 5–8
shared) with the factorizer kept. Counter-evidence: 16 unmirrored → 8
mirrored gained ~+19 at 11.7M rows. Elo: GUESS +3..+10 at 50k nodes; NPS
−1..−3%. Effort M (`KING_BUCKETS`, `view()`, `FEATURE_SET` bump, re-prepare the
corpus: 21 min, 62 GB). Training ≈ 670 s/epoch.

**N6. Threat/attack input features (Stockfish "FullThreats").**
Stockfish's largest architecture gain of 2025 (LTC ≈ +14 in its own units).
Feasible with cozy-chess attack generation but each evaluation must recompute
full attack sets (no make/unmake), the transformer grows ~10× in parameters,
and the CPU trainer would need ~50 min per epoch and > 120 GB of row memory.
Park until N2 and a GPU path (N7) exist. Effort XL.

**N7. A GPU trainer path (bullet).**
Jakgro rows map one-to-one onto bullet's text input; a ~100-line export
converter plus EMA/QAT equivalents would be needed. Worth it only if N3/N5
show the CPU trainer, not the data, is the limit. Cost: a rented GPU and
pipeline divergence.

**N8. Checked and fine.** Mirroring and bucket mapping are correct and tested;
quantisation bounds are proven by the loader; the refresh-cache gate and the
parent/sibling start-state choice are measured optima; VNNI and weight
prefetching were measured slower; output buckets match Stockfish's; int8
transformer weights are not viable at this scale. Record the shipped net's
int16 accumulator extremes once (cheap) before any width change.

### 3.7 NNUE data and training recipe (D)

The corpus: 224M rows after deduplication — 131M from 10k-node self-play
labelled by the run-42 128-unit net (a teacher ~85 Elo weaker at 50k nodes
than the current head: +44.6, +15.2, +25.9 since) and 93M from 20k-node play
labelled by the 512 nets; openings are the 2048-position random-walk book plus
8–16 random quiet plies; labels `0.25·outcome + 0.75·sigmoid(K·score)` with K
fitted once to the handcrafted evaluation; 26% draws (self-play at these
depths is very decisive); no Syzygy in generation; the Aggression 75 side
records the styled candidate's score, not the objective one.

**D1. Regenerate with the current head as teacher and Syzygy on both sides; replace the 131M run-42 rows.**
Every public pipeline regenerates with each new net; Jakgro never has. Tables
on cost −0.24% NPS and label every ≤ 5-man root with the truth (sigmoid
saturates to the outcome), improve 6–8-man labels because 10–20k-node trees
reach tablebase leaves, make outcomes exact once a game enters tablebase
range, and shorten games (178 vs 202 plies in the gauntlet). Today 32.8% of
≥ +250 rows at ≤ 8 pieces are draws and the label stays 0.71 against a truth
of 0.5. Elo: GUESS +5..+20 at 50k nodes for a continuation on ~130M fresh
rows replacing the 10k rows; +10..+30 for the from-scratch gen-2 (D4); at
least as large at long controls (endgame conversion is where the 120+1 points
leaked). Effort S engineering (`generate_nnue_corpus.py` gets a
`--syzygy-path` passthrough; `gen.sh` points at the head binary and
`artifacts/tb/3-4-5`); ~16 h machine time at 20k nodes (~9 h at 10k). Risk:
style fixtures re-pin for any new net. Test: 4096 games at 50k nodes with
tables on both sides (deployment configuration) and once without; 50 ms;
2048-game 75-vs-0 forcing ratio.

**D2. Tablebase-relabel the ≤ 5-man rows of the existing corpus and dev split.**
~14.6M rows (6.5%): draw → outcome 0.5, score 0; win/loss → 1/0 and ±2000.
Re-prepare (21 min) and continue 8 epochs (64 min). Also makes the dev split
honest in tablebase range. Elo: GUESS 0..+6 at 50k nodes with tables on in the
screen, +3..+10 with tables off. Effort S (~2.5 h wall). Add two game-free
probes: TB-labelled ≤ 5-man dev MSE, and an "erosion" set from the gauntlet
PGNs (positions where Jakgro reported ≥ +2.5 and the opponent or the tables
say equal) scored with `nnue-data score`.

**D3. Settle label depth with an equal-volume, same-teacher comparison.**
The "20k-only" arm (−4.3) confounded depth with 2.6× less data and a
different position distribution. Public evidence says deeper is not
automatically better (Stockfish: depth 9 "sweet spot"; Viridithas/Stormphrax
24–25k soft nodes). Design: 40 groups at 10k (≈ 3.75 h) and 40 at 20k
(≈ 7.3 h) with D1's setup, continue the shipped net on each plus the existing
20k rows, 4096 games at 50k each. Elo of deeper labels: GUESS −3..+8.

**D4. From-scratch long schedule on the refreshed corpus with factorizer, QAT and EMA from epoch one.**
The only from-scratch 512 run used 131M rows and no factorizer; 80 epochs beat
30 by +18.7; the control continuation plateaued at epoch 1 and a 2e-4
continuation exported nothing because the dev-loss-must-improve rule rejects
early epochs. Stockfish's retrains are 800 epochs at 4.4e-4 with gamma 0.995,
a long anneal rather than 8 epochs at 1e-4. Cost ≈ 10.6 h now (one night),
~4 h after N2. Elo: GUESS 0..+15 over the best continuation. Risk: seed
variance — replicate the winner once.

**D5. Cheap label-side arms (each ~1 h training + 4 min match).**
(a) λ by piece count (0.25 middlegame, 0.5 at ≤ 8 pieces) — only after D1/D2,
since the ≤ 8-piece outcome is itself wrong 33–43% of the time today; (b)
refit K to the current 20k-node scores (the fitted K against the handcrafted
evaluation; `fit_scaling` in `tune.rs` does it) — diagnostic, GUESS −5..+5;
(c) loss exponent 2.6 (nnue-pytorch), one line; (d) a λ schedule across a
from-scratch run (needs outcome and teacher stored separately, +4 bytes per
row). Prior: global λ 0/0.1/0.25/0.5/0.75 only; 0.25 (= Stockfish's end
lambda 0.75) stays.

**D6. Won-ending shuffles and resign calibration in generation.**
`--resign-score 800` sits at the compressed network's ceiling (KQvK ≈ +800), so
trivially won games run to move 100–130 (1.7% of rows are ≥ move 60 at
|score| ≥ 800). Tables on fix this inside five men; lower the resign score to
600 for the rest. GUESS 0..+2 Elo, +5..+15% generation throughput. Effort S.

**D7. Objective labels for the Aggression 75 side.**
When `choose_styled_candidate` picks an alternative the recorded score is the
styled move's score (up to 16 cp below objective ordinarily, 67–120 for
verified sacrifices). Measure the selection rate first (`debug on` prints
`selections=`; section 1.3 suggests it is confined to shallow iterations, and
at 10k–20k nodes the final iteration is shallow); if above ~5% of moves, emit
and parse the objective score. GUESS 0..+4. Effort S–M.

**D8. Opening diversity and exit checks.**
No Chess960, so no DFRC; a second, realistic book (8moves_v3 converted to
4-field EPD) and a Viridithas-style exit check (discard games whose first
recorded score exceeds ±500) are cheap hygiene. GUESS 0..+5. Effort S.

**D9. Checked and fine.** Filtering (not in check, no capture/promotion best
move) matches public practice; deduplication is sound; λ 0.25 agrees with
Stockfish's end lambda; dev-loss epoch selection tracked Elo across the
overnight arms; four concurrent trainings fit in memory; the endgame
(`artifacts/research/endgame-book.epd`, 864 six-to-eight-man entries) and won
books exist as secondary test channels.

**Proposed ladder (hours on this host; screen = 4096 games at 50k nodes, tables on both sides).**
0. (1 h) Syzygy passthrough in `generate_nnue_corpus.py`; stage the head
   binary; `--resign-score 600`.
1. (2.5 h) D2 relabel → prepare → 8-epoch continuation → screens and probes.
2. (11 h, background) D1/D3 generation: 40 groups at 10k, 40 at 20k.
3. (2.5 h) Two continuations (10k-new vs 20k-new) → pick the depth.
4. (16 h, background) Finish ~130M rows at the chosen depth.
5. (11 h, overnight) D4 gen-2 from scratch; screens at 50k, 50 ms, style, Gate
   2, 1000-game gauntlet; re-pin the style fixtures if adopted.
6. (M, parallel) N2 if more than one gen-2 iteration is planned.

### 3.8 Speed (P)

Cost model from the profile: ≈ 1,400 cycles and ≈ 3,000 instructions per
node at IPC ≈ 2.1. All items below must leave the depth-13 signature
unchanged (24,857,646 / 26,995,867 / 49,930,907 nodes at Aggression 0/75/100)
and are verified with `tools/measure_nps.py` or the paired
`measure_search_efficiency.py`; the Elo slope here is about 115·log₂(speedup)
at 50 ms, flatter at long control.

**P1. Prefetch the child's table bucket before the move is made.**
Today `context.table.prefetch(child_key)` is issued after `board.clone()` +
`play_unchecked_with_piece` + `repetition_key`, leaving a 50–100-cycle window
before the child probes; L3 latency on this host is 100–130 cycles and DRAM
(under match load, or Hash ≥ 64) 300–350. Compute the child key without
playing — `hash_without_ep ^ Z[attacker][us][from] ^ Z[placed][us][to] ^
Z[victim][them][to] ^ castle deltas ^ side` — and prefetch (a) the hash
move's child right after the hash move is validated, before static evaluation
/ RFP / razoring / null / singular (a 300–600-cycle window for the first child
of ~45% of interior nodes); (b) the null child (`hash ^ side`) after the
evaluation; (c) at pick time after the futility/LMP decisions. The fork keeps
`mod zobrist` private, so this needs a fork method such as
`Board::hash_without_ep_after(mv, piece)` or a duplicated 6.4 KB key table
pinned by a test against `Board::hash()`. A mispredicted key (en passant,
castling) wastes one line, never a bug. Evidence: 1 MiB vs 64 MiB NPS = 12.7%
ceiling; the current post-make prefetch bought +2.4% under 64-engine load.
GUESS +4..+8% NPS at Hash 64 or under load, +1..+3% at Hash 16 idle. Effort M.
Must be measured under load (64 concurrent `perf_driver.py` instances) as
well as idle.

**P2. Reuse `CheckMasks` for every emitted candidate; tighten the king rule.**
`emit_candidate` passes `None` masks, so every hash move, promotion, capture
and killer pays the full `move_gives_check`; `may_give_check` returns `true`
for every king move; in quiescence quiet-check mode king moves are not
intersected with any mask and the fork's `king_danger()` sweep runs. Build the
masks lazily on first emit and pass them; replace the king rule by `castling
|| discovery_candidates.has(from)`. Evidence: `move_gives_check` 3.9%.
GUESS +1.5..+3% NPS. Effort S (~30 lines). Extend the existing
never-reject-a-real-check tests to king moves and captures.

**P3. Evaluate the stand-pat before probing in quiescence when the table is DRAM-resident.**
The NNUE compute (~260 cycles) can overlap the in-flight prefetch; wasted
evaluations (table cutoffs + table-supplied evaluations ≈ 17% of quiescence
nodes) cost ~43 cycles per node against up to ~200 saved under DRAM latency.
Values are identical whichever source wins, so the tree is identical. GUESS
+4..+8% at Hash ≥ 64 or under load, 0..−1% at Hash 16 idle — gate on
`size_mib() >= 64` or accept after a loaded measurement. Effort S.

**P4. Pack quiets into one `u64` and sort a single key.**
The quiet sort is a two-level comparator on 16-byte `QuietMove`s (6.3% of
cycles); `(score << 15) | (0x7FFF − move_key)` reproduces the order exactly and
sorts branchless. GUESS +1.5..+3%. Effort S–M. Incremental selection is not
recommended: all-nodes emit every quiet (pruned ones too, the index must
advance), so selection would be quadratic there.

**P5. Remove duplicated per-move work.**
`HistoryMove::from_board` scans `piece_on` twice per emitted move although
`metadata.attacker` is known; the hash move's legality is checked in `negamax`
and again in `MovePicker::new`; `MovePicker::new` clears storage already
cleared by the context. GUESS +0.5..+1.5%. Effort S.

**P6. Whole-binary `-C target-cpu=x86-64-v3`.**
Measured +2.3% fixed-node NPS (`native` −3%); also makes `occupied().len()`
popcounts single instructions. Effort S (`.cargo/config.toml`). Risk: SIGILL on
pre-Haswell hosts; a separate release artifact. PGO stacks at +1.4–2.3% and
was judged not worth the build complexity.

**P7. Smaller items.** `update_pv` walks the table with unprefetched probes
after an exact cutoff child (< 1%; lazy resolution would not be
tree-identical); expose the fork's branchless `captured_piece_on`; lazy
`gives_check` for SEE-good, non-delta-prunable captures in quiescence;
telemetry increments ≈ 1%. Not recommended: playing the move before pruning to
widen the prefetch window (30–50% of emitted shallow quiets are pruned),
make/unmake (snapshot = board size, measured), lazy SEE at pick time (changes
capture order), removing the quiescence stalemate probe (0.7%, a search
change).

**Combined stack P1+P2+P4+P5 (+P3 under load): GUESS +8..+15% NPS under match
load, +4..+8% single-threaded at Hash 16 → +6..+15 Elo at 50 ms.**

### 3.9 Lazy SMP (M)

Measured: 2 threads +69.0 [+62.5, +75.6], 4 threads +122.8 [+112.9, +132.8]
over 1 at 50 ms (Hash 16); NPS linear to 32 threads (section 1.5). Helpers
are spawned per `go` (no pool), run the objective (Aggression 0) policy and
never the styled root, start every move with a cold `MoveOrdering` while the
main thread inherits the game-carried one, use odd/even depth schedules and a
root rotation for diversity; only the main thread reports and decides; no
voting.

**M1. Warm-start helpers from the carried ordering.** Pass `carried.clone()`
(≈ 630 KB) to each helper. GUESS +2..+8 at 4–8 threads, 0..+5 at 16. Effort S
(~10 lines). No effect on the single-thread path. Test: 8T vs 8T same binary,
50 ms, 4096 games at concurrency 5 (~70 min).

**M2. Hash advice and cap for multi-thread users.** At 8 threads (~16 Mnps)
a 16 MiB table turns over in ~60 ms, five times per move at 10+0.1; the
single-thread NPS loss from larger tables (9–11%) is an L3-residency effect
that does not transfer to a busy machine. Measure 16 vs 256 MiB at 8 threads
and 10+0.1 (1280 games, ~2 h at concurrency 5) and publish a per-thread
recommendation in the README; raise `MAX_HASH_MIB` (T7). GUESS +5..+20 at 8
threads and 10+0.1.

**M3. Diversification at ≥ 8 threads.** The depth plateau at 8 threads in
section 1.5 and the rotation `index + 1` wrapping modulo the alternative count
suggest helpers coincide at 16–32 threads; Stockfish relies on timing plus a
per-thread aspiration delta and dropped depth skipping as a simplification.
Port `ASPIRATION_INITIAL + k·(index % 8)` for helpers; try dropping the (2,2)
schedule. GUESS ±3 at 8 threads; only worth a slot once 16-thread scaling is
measured.

**M4. Thread voting / adopting a helper's deeper completed iteration.** Odd
helpers at (2,2) and all helpers without the styled-root cost often complete
depth d+1 when the main thread stops at d; that result is discarded.
Stockfish's depth- and score-weighted vote measured +1.9 (8T LTC) and +6.1 (4T
STC) over its older rule. Minimal safe form: adopt a helper result only when
deeper and naming the same move, or when the main iteration aborted and a
helper completed that depth with a move the style would accept. GUESS +2..+6
at 8 threads, +3..+8 at 16. Effort M. After M1/M2.

**M5. Persistent helper pool and buffer reuse.** Per-`go` spawn plus ~1.3 MB
of fresh per-worker allocations measured +3.5 ms first-`info` latency at 8
threads (7% of a 50 ms move → GUESS +3..+8 Elo at 50 ms/8T, ~1% at 10+0.1,
≈ 0 at 60+0.6). Measure the spawn breakdown (`strace -c -f`) before building;
keep the `threads ≤ 1` inline path bit-identical. Effort M.

**M6. Helper policy vs table semantics.** Helpers write Aggression-0 results
into the Aggression-75 table that `start_search` otherwise treats as
profile-specific; under SMP the realised style may drift from the single-
thread fixtures (no SMP PGN was retained, so this is unmeasured). Contract
fidelity rather than Elo (GUESS −5..+3 for making helpers styled). Measure the
75/0 ratios from the M1 PGN.

**M7. Store without re-read, root ranking of Syzygy under SMP, `tbhits`.**
Stale-snapshot stores can overwrite a fresher deeper entry (acknowledged;
GUESS < 1 Elo below 16 threads); `tbhits` reports only the main thread's
count (cosmetic). ABDADA-style exclusion is literature-only (GUESS 0..+15 at
16–32 threads, 0 at 4); not before M1–M4.

**Measurements to schedule.** 8T vs 1T at 50 ms (4096 games, concurrency 5,
~70 min; keep the PGN); 4T and 8T vs 1T at 10+0.1 (1280 games each); Hash at
8T (M2); add `--candidate-threads/--baseline-threads` to `run_sprt.py` to
retire the `JAKGRO_FORCE_THREADS` wrapper.

### 3.10 Endgame and draw knowledge (E)

The Syzygy integration matches Stockfish's probe conditions, root ranking
(including the repetition-triggered DTZ ranking) and score band, and
deliberately keeps probing inside the tree when the root is covered (which is
why it converts 800/800); draw aversion in the styled root is inert for draw
scores; mate-distance pruning exists; insufficient material is detected
(KNNvK excluded).

**E1. Add the 6-man tables (WDL first).**
`SyzygyProbeLimit` already allows 7 and `covers` follows the largest table
found. WDL-only (72.8 GB, fits the page cache) suffices for in-tree steering;
DTZ (87.4 GB) adds root ranking at six men. The audited phantom-advantage
classes (KRPPvKR, KBPPvKB, KBPvKBP, KNPPvKN, KQPvKQP) are six-man. Public:
Stockfish 10 at 10+0.1, 6-man +20 vs none, 5-man +9. Elo: GUESS +10..+25 on
the endgame book over 5-man, +1..+4 from openings at 50 ms, +2..+8 in the
30+0.3 gauntlet. Effort S (download, two matches); pre-warm the files and add
a forced control poll after a probe (E3) before 50 ms matches. Test:
`run_sprt.py --candidate-syzygy-path 3-4-5:6 --baseline-syzygy-path 3-4-5
--openings artifacts/research/endgame-book.epd`, then openings, then
`gauntlet-syz.sh` with `tb_probe.py --max-pieces 6`. Verify pyrrhic-rs's 6-man
DTZ root probe against python-chess on one FEN first (only 3–5-man has been
exercised).

**E2. Rule-50 damping of the static evaluation.** See Q2.

**E3. Two hygiene fixes in the tablebase block.** (a) The WDL probe precedes
any no-legal-move check, so a side checkmated by a zeroing move inside
tablebase range scores `−TB_WIN + ply` rather than mate and `mate_settled`
never fires; check `in_check && !legal_move_exists` first in `negamax` and
`quiescence`. (b) Force a control poll after a tablebase hit (Stockfish resets
`callsCnt`) to protect cold 6-man probes at short controls. GUESS 0..+1.
Effort hours; node-identical without tables.

**E4. Draw-score randomisation.** See Q7.

**E5. Drawish-ending scale tables on the network output — not now.** Narrow
forms are covered by 5/6-man tables; two prior scalings were neutral or
negative; Stockfish carries none with NNUE. Let D1/D2's tablebase labels carry
this. GUESS −3..+3.

**E6. Single-root-move early exit.** When `rank_root` leaves one move, or one
legal move exists, the full soft budget is still spent. GUESS 0..+2 from banked
time. Effort S.

**E7. Measure the 6-man share of the phantom advantage.** `tb_probe.py
--max-pieces 6` over the existing 1800 + 2000 gauntlet games once the 6-man
WDL is on disk decides E1's ceiling; the gauntlet's draw adjudication now ends
games the skill-limited opponents used to lose, so gauntlet deltas understate
tablebase value against weak opponents.

### 3.11 The Aggression profile at equal style (A)

Inventory of what Aggression 75 changes against 0: a third check extension
per line (`max_check_extensions = 2 + aggression/50`); the forcing-order bonus
(every quiet check and attacking pawn push above every history-ordered quiet
at every interior node); the move-count/LMR exemption for attacking pawn
pushes (`protected`); and the styled root (threshold probing, ≤ 2
verifications inside `STYLED_ROOT_*` node caps, margins 16/20/67 cp). The
quiet-check budget in quiescence, the reverse-futility margin and the
capture-history weight are identical at 0 and 75; the evaluation is objective
at every profile; helpers never run the styled root. Measured at 50 ms:
Aggression 0 +45.3, styled root off +38.5, probe-but-play-objective +6.7,
forcing bonus off +5.9, check-extension cap 2 +5.5/−0.8. At 10+0.1: Aggression
0 +14.9 [+5.4, +24.5], styled root off +1.7 [−8.1, +11.5]. Section 1.3 adds:
the styled root selects nothing beyond depth 6.

**A1. Stop the styled pass once it cannot complete.**
The budget is flat 2048/4096 nodes from depth 6 (`styled_root_node_limit`);
from depth 9 every iteration exhausts it and returns the objective move
(section 1.3) while spending 1.5–4.6% of the nodes at depth 12–16 and 10–18%
at 50 ms. Options, in order of fidelity to the current style: (a) skip the
styled pass after two consecutive exhausted iterations unless the objective
best move changed; (b) cap the styled budget at a percentage of the objective
pass's nodes for that iteration (floor 128, ceiling unchanged) so shallow
iterations keep their table-warming effect; (c) the parked shallower-budget
patch (+25.6 at 50 ms, +12.4 at `1.0+0.01`, +3.0 at 10+0.1; checks ratio
falling to 0.921 at 10+0.1 — the reason it was parked). Elo at equal style:
GUESS +5..+15 at 50 ms, +2..+8 at 10+0.1 for (a); style unchanged at depth ≥ 9
by construction because nothing is decided there. Effort S. Risk: 20k-node
knife-edge fixtures (`standard-knight-e4-investment`,
`standard-opposite-castle-storm-deeper`) may flip under (b). Test: Gate 1 and
Gate 2 with the style bootstrap; fixed-node 4096 games to isolate the budget.

**A2. Forcing-order bonus as a bounded, SEE-gated history term.** See O3.
This is the largest remaining interior coupling and the one whose removal
measured style-neutral.

**A3. Redesign the styled root for depth.**
A probe must prove an upper bound on a child whose objective-pass entries are
lower bounds, which a depth-(d−1) all-node cannot do in 2048 nodes beyond
depth ~9. The honest fix is to verify candidates inside the root PVS with a
window `(threshold − 1, alpha + 1)` so the same-depth exact score comes from
the objective pass, and to make the margins depth-aware (16 cp at depth 20 is
a different error than at depth 7). Effort L; GUESS Elo −5..+5 with style
restored at long control. Decide after A1's diagnostic matches.

**A4. Cache the root-constant styled work.**
`root_complexity_bonus`, `root_interest`, `exchange_risk_on`,
`sacrifice_hint_score`, the root `style_snapshot` and the sterile-
simplification snapshots depend only on the root board and move yet are
recomputed every iteration and every aspiration retry (two `features::extract`
and one `exchange_risk_on` per root move per pass). Tree-identical, no re-pins.
GUESS +0..+3 at 50 ms, 0 at long control. Effort S. Free rider.

**A5. Third check extension only for SEE-safe checks.** See S10.

**A6. Fix the `== 75` literals.** `candidate_risk_margin` and
`selection_interest` branch on `aggression() == 75` (margin 16 vs 26 at 76);
generalise before any profile tuning. Effort S; no Elo.

**A7. Measure the long-control style cost properly.** Self-play 75 vs 0 at
10+0.1 with 4096 games (±6) replaces the current ±10 figure; attribute with
the switchboard bits 1 (styled off), 8 (forcing off), 2 (check cap) at 10+0.1
(24 min each); then 30+0.3 with style ratios from the same PGN. The external
gauntlet (±18 per 1000 games) is too noisy for a 10–15 Elo difference; use it
once as a transfer check.

**A8. Reduce the fixture tax (development velocity is Elo).**
A search change today forces: rebuild → seven regression records with exact
scores at 20k nodes → re-pin → personality, standard and sacrifice suites at
three profiles (hundreds of fixed-node searches, each also searched restricted
at Aggression 0) → judge knife-edge flips → edit EPDs → update SHA locks in
`dual-channel-null-contract.json` → gate harness → archive. Replacements:
score tolerance (±25 cp, mates exact) plus legality and the twice-run
determinism check (the template exists in `tests/null_move_regression.rs`);
per-category hit rates at three budgets with floors instead of per-record
exact moves, keeping exact gates only for objective truths (mates, hanging
pieces, forced defences, rejected unsound sacrifices); the forcing/check
bootstrap folded into `run_sprt.py` output as the statistical style gate
(≥ 0.95 checks, ≥ 0.97 forcing over 4096 games); the 48-game Elo channels
dropped in favour of the SPRT; SHA locks moved out of CI into the archive
step; one long-search fixture tier (≥ 1M nodes or depth 14) for the two or
three motifs the project actually promises, because 20k nodes samples depth
7–8 and tournament play is at 17–20.

### 3.12 Testing and tuning methodology (X)

Current state: `selfplay` charges wall time around the UCI round trip with a
250 ms grace (25% of the base at `1.0+0.01`), never forfeits fixed-time games,
sends no `Move Overhead`; adjudication is two-sided resignation at ±800 for
four moves (74% of games end that way at 50 ms), draw from move 80, cap at
200; the 2048-position book is 48 seeds plus 2–10 random quiet plies
(positions with the white king on d2 and a black rook on a6 in the opening);
`run_sprt.py` plays one fixed batch and evaluates the LLR once (not
sequential), defaults `elo0 0 elo1 10`, and cannot exceed 4096 games per run
because `--random-plies` is not forwarded; concurrency 88 on 48 cores × 2 SMT
means every search shares a physical core; there is no `nodestime`, no `bench`,
no tunable options and no SPSA driver (the 24-constant tune left no tool); CI
runs the fixed-node fixtures unoptimised (no `[profile.test]`), so
`aggression_profile` alone takes 175 s; the absolute gauntlet anchors Stash
v25 at 2744 while CCRL Blitz lists Stash 25.0 at 2933 ± 18, and the four
anchors disagree by ~110 Elo.

**X1. Make 10+0.1 the acceptance gate with a real sequential SPRT and
hypothesis-chosen bounds.** Batch loop in `run_sprt.py` (512 games per batch,
forwarding `--random-plies`/seeds or cycling book offsets), LLR after each
batch, stop at ±2.944 or `--max-games`, record that the estimate is
post-stopping. Bounds: gainers (0, 5) — the project's effects are +5..+20, so
fishtest's (0, 2) is unaffordable; simplifications (−5, 0); speed and refactor
non-regression (−3, +1). Expected cost at σ_pair ≈ 68 Elo: ~2200 games when
the truth is at a bound, ~3200 at the midpoint — 13–20 min per decision at
10+0.1. Validate: same-binary control at 10+0.1 (4096 games, must straddle 0
at ±5–6); re-measure 4–6 known 50 ms results at 10+0.1 (switchboard bits 8,
128, 16, 32, 256) and report the correlation — if half differ by more than 6
Elo, 50 ms is demoted to a screen. Effort S–M.

**X2. A realistic opening book, re-baselined.** Convert `8moves_v3.pgn` (on
disk) to 4-field EPD for `selfplay`; keep the old book as a secondary channel
for one series to calibrate. Section 1.8 shows the consequence of the book
choice: 61% draws with `8moves_v3` at 10+0.1 against 25% with the random-walk
book at 50 ms, so game counts per Elo change too. The same book seeds NNUE
self-play, so evaluation and tests share today's bias. Effort S.

**X3. A node-based virtual clock (`nodestime`).** Elapsed = nodes/nodestime so
the time manager (soft/hard, forecast, volatility hold) is exercised
deterministically regardless of CPU contention; removes the SMT-sharing noise
and the bias that memory-stall-heavy code looks better when a sibling thread
hides latency; lets concurrency reach 96. `SearchControl` already polls on a
node cadence and `allocate_time` is unit-agnostic; the arbiter charges clocks
from the reported `nodes`. NPS-changing patches still need real time. Effort
M. Validate: node identity with it unset; a candidate pair at 10+0.1 real vs
nodestime-equivalent within noise; repeated runs identical.

**X4. Fix the absolute anchor.** Replace the Stockfish skill-level ladder (a
random-blunder error model a tactical engine exploits unlike CCRL's pool) with
a version ladder of real engines bracketing Jakgro, all GPLv3 C with `make`
(public CCRL Blitz ratings): Stash 28.0 3083 ± 18, 30.0 3153 ± 17, 32.0
3240 ± 14 (repository already checked out), Weiss 1.3 3136 ± 15; 400+ games
each at 30+0.3; implied rating = anchor − measured Elo, weighted by both
errors. Cheaper drift check: one frozen opponent (Stash 30.0) at 10+0.1, 2048
games weekly (~12 min), plus the frozen Jakgro bases as a second trend line.
Effort S.

**X5. Tunable parameters, a generic option passthrough and an SPSA driver.**
A `tunable` cargo feature turning the ~24 search constants and the time
constants into atomics with `option name … type spin` lines (release keeps
`const`); `--candidate-option/--baseline-option` in `selfplay`/`run_sprt.py`; a
~200-line Python SPSA (fishtest schedule) over `selfplay` in pair-balanced
mini-batches, noise-free with X3. Order of families: time manager at 10+0.1
and 30+0.3 (nothing tuned above 50 ms; the clock patch was worth +18.8), then
the LMR table and history thresholds, then margins at depths 7–12, styled-root
budgets last and only under style constraints. Cost: 60k games ≈ 6 h at
10+0.1. Never ship a tune on its own run; confirm with X1. Effort M.

**X6. A `bench` command and a cheaper CI.** `bench [depth] [hash] [threads]`
printing nodes and NPS over a fixed suite at Aggression 0/75/100 replaces
`depth_driver.py`, gives a one-line tree-identity check for "no functional
change" commits, enables OpenBench-style speed tests and a `Bench:` trailer
per commit. Add `[profile.test] opt-level = 3` so fixtures stop running at
opt-level 0 (GUESS 3–10× faster `aggression_profile`). Split pins into
contract pins (safety, anti-sacrifice, `maxloss`) that must hold and
descriptive records regenerated by a script and reviewed as a diff. Effort S.

**X7. Reporting discipline.** Accepted-patch estimates are winner's-curse
inflated (selected because they cleared ~2σ; series totals fall short of the
sum of patches); a null candidate exceeds +5 in one of twenty screens and
beats a +2 control by ≥ 4 in three or four — the "+5.9, +4 above control,
unresolved" rows are what that looks like. Pre-register bounds, report LLR and
"continue", run same-binary controls at every regime used (only 50 ms has
one), and do not choose arms by point Elo over a control that itself read
+8.1 [+2.1, +14.2].

**X8. An endgame-entry channel.** Self-play from the random-walk book rarely
reaches ≤ 8 men and ends 74% of games by resignation, so conversion changes are
structurally under-rewarded (Syzygy: +3.9 from openings vs +39.8 from
endgame entries vs a 17–18 Elo ceiling in external games). Keep
`endgame-book.epd` / `won-book.epd` as a standard secondary test for evaluation
and tablebase changes; consider Syzygy adjudication in the arbiter so drawn
≤ 5-man positions are not played out to repetition.

**X9. Arbiter details.** Send `Move Overhead` explicitly; forfeit-check fixed-
time games too (currently never); `isready` after `ucinewgame` (needed for
foreign engines); record σ_pair and the decisive rate at 10+0.1 (no arbiter
JSON survived) to fix the SPRT game budgets.

### 3.13 UCI and engine lifecycle (U)

**U1. A negative `wtime`/`btime` drops the whole `go` and forfeits the game.**
`parse_u64` fails on `-37`, the command is silently ignored and no `bestmove`
is ever sent. cutechess subtracts without clamping and sends negative clocks
(either side's) when a `timemargin` is set; fastchess clamps to zero, which is
why 1800 gauntlet games never hit it. Parse as i64 and clamp; skip unknown
`go` tokens instead of failing. Elo: each incident is a lost game (−3..−7 per
1000 games per incident, GUESS); EVIDENCE 0 under fastchess. Effort S.

**U2. `Hash` above 1024 is silently refused and the engine plays with the previous table.**
`set_hash_size_mib` returns `OutOfRange` and the message appears only in
debug mode; cutechess additionally refuses to send values above the
advertised maximum. CCRL allows 4-CPU entries up to 2048 MB. Raise
`MAX_HASH_MIB` and clamp like every other spin. GUESS −15..−40 for anyone
hitting this at 40/15. Effort S (one constant, `selfplay.rs`, a session test).

**U3. The first search after any table allocation re-zeroes the whole table on the clock.**
`TranspositionTable::new` leaves `evaluation_profile` at the sentinel and
`clear()` does not set it, so the first `start_search` sees a profile mismatch
and runs `discard_entries()` over a table `ucinewgame` just cleared (~0.1–0.4 s
at 1 GiB; once per process, and after every `Hash`/`EvalFile`/`Use NNUE`
change). Have `clear()` store the current profile. Elo ≈ 0 at CCRL; a wart for
GUI users who restart per game. Effort S.

**U4. `Ponder` is not advertised, so GUIs never ponder.**
cutechess forwards the ponder move and sends `go ponder` only when an `option
name Ponder` line was seen; fastchess has no ponder support at all. Jakgro's
ponder path itself is correct and withholds results properly. Emit `option
name Ponder type check default false` and accept it. GUESS +20..+40 in
ponder-on events versus not pondering; 0 under CCRL (ponder off). Effort S;
run one cutechess match with `ponder=true` both sides and check for premature
`bestmove` warnings.

**U5. Move Overhead and the period boundary.** The effective guard is
`overhead + reserve` (≥ 110 ms above 2.2 s remaining, only 15 ms below ~200 ms
remaining); recommend 30–50 ms for cutechess/fastchess at `timemargin=0` and
consider 30 as the default (it costs ~3 ms of soft time per move). At
`movestogo ≤ 2` the hard limit equals `usable` (see C4). Test: 1000 fastchess
games at `tc=120+1` and `tc=40/900` with `timemargin=0`, count forfeits.

**U6. Hard failures become forfeits.** `panic = "abort"` makes the
`catch_unwind` in the worker useless in release; a rejected `position` leaves
the previous position and the next `go` plays from it; `stop` before depth 1
plays the alphabetically first legal move. Only bugs or GUI bugs trigger
these (1800+ clean games); keep "first legal move of the current position"
as the fallback rather than `0000` where possible. Effort S.

**Operator settings for CCRL-like play until U2 lands:** `Hash 256`, `Threads
1`, `Move Overhead 30–50` with margin-0 GUIs, `SyzygyPath` to the 3-4-5 (and
6) tables, Aggression at the project's profile, do not restart the process
per game.

## 4. Measured and rejected: do not retry in the same form

Collected from `docs/tuning` and the research artifacts so that nobody
re-spends a test slot. Each entry names the regime; many were only measured
at 50 ms or fixed nodes, which is itself the reason some reappear above as
"re-measure at 10+0.1".

- Reading a stored table bound in place of the static evaluation, three
  forms: −19 to −30 (50 ms). See Q9 for the open question.
- Storing bounds after pruned or reduced moves without re-search: −19.3.
- Hash move first in quiescence: −4.9 and −3.4; mechanism understood (1.7% of
  quiescence nodes can benefit, the classification cost is paid everywhere).
- Full-weight continuation history bundled with a follow-up table: +1.3
  (unattributed, see O4); countermove: −1.6; killers reduced instead of
  exempt: −1.4; fractional history LMR: −1.1; no malus for pruned quiets: +2.3
  (null).
- Narrower aspiration windows at 50 ms: −10 (16/30) and −0.3 ply (30/40);
  cut-node reductions at 50 ms: −12 with +6% nodes at depth 8 (see S2, S9).
- ProbCut from depth 5 at 50 ms: −3.2 [−9.4, +3.0] (see S7).
- Double and negative singular extensions at 50 ms: −0.5 (see S6).
- Reverse-futility to depth 9 and quiet futility to depth 8: −2.3; the four
  depth caps raised together: +1.9% nodes at depth 13 and no match (see S5).
- The Stockfish move-count formula: searched more nodes here (see S11).
- Root LMR: +2.5 [−3.5, +8.4] (null); root ordering by previous scores: −7.1.
- Correction history, four forms (pawn key on margins +1.1; pawn + material
  counts on stand-pat −8.1 pre-NNUE; pawn key on the pruning evaluation −6.5
  and −1.2): see Q1 for why none is the canonical form.
- Material scaling of the network score: +3.7 and −5.9 (noise); hand endgame
  scaling of the handcrafted evaluation: −9.9; damping toward the fifty-move
  draw: +6 within noise, parked for a fixture flip (see Q2 for the long-control
  case).
- Stability-scaled soft time with a node-effort term at `1.0+0.01`: −0.7;
  repeatedly extending volatile searches: −9.2; best move of an abandoned
  iteration alone: +1.7 (see C2, C3).
- Check-extension cap 2: +5.5 and −0.8 over 8192 games; attacking-pawn-push
  exemptions off: −0.8; sacrifices given only the ordinary margin: −2.2
  (style gone); ordinary margin 0: +4.0 (style gone); the parked shallower
  styled-root budgets (see A1).
- NNUE: λ 0.5 / 0.75 global (−7.9 / −29.9); 20k-only labels (−4.3, confounded,
  see D3); K 1.2 (−14) and 0.7 (noise) at 128 units; a 256-wide net at the
  128 era (noise, −10..−20% NPS); VNNI output kernel and weight-row prefetch
  (slower); ungated refresh cache (−3% endgame NPS); whole-binary
  `target-cpu=native` (−3%).
- Make/unmake instead of board clones (a snapshot costs as much as the copy);
  early table prefetch at the same placement (0.981×, later +2.4% under load).
- PGO: +1.4..2.3% on the NNUE head (was +10.7% pre-NNUE); not worth the build.
- Time-gating the styled pass on the remaining clock: +10.9 but checks 0.802.

## 5. A program

Ordered so that each phase makes the next one's measurements trustworthy.
Phases 1 and 2 are a few days; 3–6 can run in parallel on this host.

1. **Measurement first (X1, X2, X6, X3).** Sequential SPRT at 10+0.1 with a
   same-binary control, the realistic book as a second channel, `bench`,
   `[profile.test] opt-level 3`; then the node-based clock. Re-measure the
   switchboard bits at 10+0.1 to calibrate how much of the 50 ms record
   transfers.
2. **Protocol and safety (U1–U4, C4, T7).** One afternoon, no Elo risk,
   removes forfeits and the hash cap, enables pondering in GUIs.
3. **Search at depth (S1–S3 → S4–S9 → S12, Q1, Q3, O1–O4, T1, T3).** Each at
   Gate 0/1/2 with style ratios; the LMR items first because every later
   reduction/pruning result depends on them; correction history and the table
   items decided at Gate 2 and Gate 3.
4. **Personality at equal style (A1, A2/O3, A4, A8).** The styled-pass skip
   and the bounded forcing bonus are small patches with large 50 ms gains and
   style-neutral mechanisms; the fixture replacements pay for every later
   search change.
5. **Network (D1 → D2/D3 → D4; N1 meanwhile; N2 before a second generation; then
   N4/N5, N3).** The corpus regeneration is the single highest-expected-Elo
   experiment in this report; the trainer rewrite decides whether
   architecture work is affordable on this host.
6. **Speed (P2, P5, P4, P1, P3, P6) and endgame (E1, E3, Q2).** Tree-identical
   speed work can land at any time with the signature check; tablebases and
   damping need the endgame channel (X8).
7. **Clock and SMP (C1–C3, C5; M1, M2, M5, M4).** After the gate is at 10+0.1,
   because every one of these is inert or inverted at 27 ms per move.
8. **Tuning (X5, S13).** Once the tunable layer exists, an SPSA pass over the
   never-tuned constants at 10+0.1, confirmed by SPRT.

## 6. Reproduction

Scripts used for section 1 live under `tools/research/`:
`depth_driver.py` (fixed-depth or fixed-time totals over the book),
`perf_driver.py` (fixed-node throughput, optionally under `perf record`),
`smp_nps.py` (throughput and depth by thread count), `styled_depth.py`
(personality counters by depth from `debug on`), `time_profile.py` (per-move
time and depth from fastchess PGN comments), and the `search_telemetry`
example (`cargo run --release --example search_telemetry -- BOOK DEPTH COUNT
AGGRESSION [HASH]`), which prints every `SearchTelemetry` counter per
position. The hash match of section 1.8 was
`fastchess -engine cmd=jakgro name=Hash16 option.Hash=16 -engine cmd=jakgro
name=Hash256 option.Hash=256 -each tc=10+0.1 option.Threads=1
option.Aggression=75 timemargin=100 -rounds 1024 -games 2 -repeat -openings
file=artifacts/rating/8moves_v3.pgn format=pgn order=random -concurrency 88
-draw movenumber=40 movecount=8 score=10 -resign movecount=3 score=600
twosided=true`. The research reports behind sections 2–4 were produced from
the shared brief in `artifacts/research/BRIEF.md` and are summarised here
rather than retained.
