# Default-profile strength: reduce late quiet PV moves

## Result

The accepted search change measures **+13.2 Elo over 3,072 games** and
**+15.7 Elo over a separate 4,096-game replication**, both at **Aggression 75,
50 ms/move** against the unchanged base. Both paired-normal 95% intervals exclude
zero and both final [0, 10] likelihood tests accept H1. Attacking root preferences
are unchanged; replication retains **101.1% of forcing moves and 101.0% of checks**.

| Accepted binary versus base, both at 75 | Games | W / D / L | Score | Elo, paired-normal 95% interval | Final LLR / decision |
| --- | ---: | ---: | ---: | --- | --- |
| Development, 50 ms | 1,024 | 407 / 246 / 371 | 51.758% | +12.2 [+0.6, +23.9] | 2.047 / continue |
| Reserved-book confirmation, 50 ms | 3,072 | 1,214 / 761 / 1,097 | 51.904% | **+13.2 [+5.9, +20.6]** | 5.943 / H1 |
| Same confirmation book, `1.0+0.01` | 2,000 | 795 / 466 / 739 | 51.400% | +9.7 [+0.5, +19.0] | 2.136 / continue |
| Replication on another existing book, 50 ms | 4,096 | 1,513 / 1,255 / 1,328 | 52.258% | **+15.7 [+9.2, +22.2]** | 9.642 / H1 |

All matches completed without recorded engine faults. The runner evaluated the
likelihood ratio **at declared game caps**, not as an early-stopping procedure.
The clocked run did **not** reach H1 despite its positive normal interval.

These are modest, relative short-control gains, not an absolute rating or a
long-control strength guarantee. Do not add the two estimates together or add
them mechanically to earlier series. The more conservative pair-aware Hoeffding
Elo intervals are **[-10.9, +37.5]**, **[-20.1, +39.7]**, and **[-5.2, +36.7]**
for confirmation, clock, and replication respectively: all still cross zero.
Both statistical reports are archived rather than choosing only the narrower one.

## What changes in the engine

`late_move_reduction` previously exempted every principal-variation (PV) node.
Late quiet alternatives at those nodes now use the existing reduction table,
with **one ply less reduction** than the corresponding scout search before
history adjustments and the final depth clamp. The first and preferred moves,
checks, captures, promotions, castling, king-zone moves, checked positions,
killers and protected attacking pawn pushes retain their existing protections.
Every reduced alpha raise is re-searched at full depth before it can update the
PV or justify a cutoff. The existing minimum remaining depth is unchanged.

The production change is one removed exemption and one three-line adjustment.
It applies to **all aggression profiles**; match evidence here concerns 75 only.
Aggression still defaults to 75, with the same fitted evaluation, three check
extensions, one optional quiescence quiet check, root-interest terms, score
margins, sacrifice verification, and draw/simplification preferences. No weights,
clock allocation, transposition policy or dependency revision is changed.

On the ten-position performance suite at depth eight, five alternating samples
and 500 ms timed probes, it searches **17.17% fewer nodes** and completes **0.5 ply
more** on average. Fixed-node throughput is **1.41% lower**. These are different
search trees, not a pure speedup. The matches, not the node saving, support the
strength claim.

An earlier PV-reduction trial in [series three](strength-series-three.md) was
rejected or narrowed partly because exact personality targets changed. This
round deliberately revisits the full rule under the requested priority:
**stronger play with attacking character, not identical historical moves**.
It is not presented as a newly invented search technique.

## Style and safety

| Match | Forcing-move rate retention | Check-rate retention |
| --- | ---: | ---: |
| Reserved-book 50 ms confirmation | 99.5% | 96.6% |
| Clocked confirmation | 101.7% | 103.8% |
| Second-book 50 ms replication | 101.1% | 101.0% |

These SAN-derived rates describe play; they are not independent labels of attack
quality or sacrifice soundness. The engine still chooses the reviewed `e5c6`
knight investment, `c1g5` open-king attack and `d2e2` queen-trade avoidance at the
default profile. It still rejects the unsupported bishop and rook sacrifices.
The original sacrifice-gate inputs and null-pruning guards are unchanged.

### Explicit changes to test expectations

**The original tests did not all pass unchanged.** Original failed reports,
review probes and before/after fixtures are retained. No root-loss cap increases.

- The starting-position move becomes `d2d4` instead of `b1c3`, with the same
  +23 cp score at 20,000 nodes. Both are admitted by the endpoint contracts;
  the deterministic search snapshot records `d2d4`. At two million nodes,
  restricted objective probes score `d2d4` above `b1c3` in both binaries.
- In the open-king fixture, Aggression 0 now also finds the attacking `c1g5`.
  Its allowed moves expand from `c3d5` to `c3d5,c1g5`; the attacking endpoint
  still requires `c1g5`. At 100,000 nodes, restricted objective scores are +111
  versus +91 cp in the candidate; at two million they are +120 versus +112.
  These are engine probes, not certified chess truth.
- At 20,000 nodes the default pawn-storm fixture chooses `d2b3`, at 2 cp loss
  against its existing objective reference. The original 45 cp cap is retained,
  and that development move is explicitly admitted. It is a lost **short-budget
  storm choice**, not relabelled as a pawn push. A new 400,000-node record requires
  `b2b4` at all three profiles with **zero** reference loss. Both base and candidate
  pass it. The separate frozen `standard-attacks.epd` targets are unchanged.
- With the objective profile also attacking in the open-king fixture, endpoint
  move differences fall from five of sixteen to four. The old 30% aggregate
  difference quota is replaced by explicit required distinctions in **initiative,
  pawn storms, sacrifice and simplification**. Mandatory tactical, defensive,
  anti-sacrifice and sacrifice checks remain enforced. This is a deliberate
  revision of the style criterion, not a claim of exact personality preservation.
- Three fixed-node scores change without changing their tactical moves:
  `win-hanging-queen` +641 to +642, `punish-central-queen` +1446 to +1455
  (depth six to seven), and `contain-lone-rook` -647 to -653 (depth seven to eight).
  Repeated cold searches reproduce all seven ordinary regression observations.
- The strict warm-table equality test now exercises Aggression 0. A separate
  default-profile test permits the equal +52 cp `b1c3`/`f1c4` move orders only
  while requiring legal five-ply PVs that reach the **same board**, equal scores,
  and fewer warm-search nodes. It does not accept arbitrary move drift.
- The checked null fixture has two winning rook captures. At depth seven,
  null off chooses `e1e2`, +1337, and null on chooses `d1e2`, +1335. A new shallow
  control requires either legal capture, a score of at least +1300, and a legal
  PV retaining the queen against a lone king. Exact null-on/off equality is
  checked at depth nine, where both return `d1e2`, +1339. Other contract fixtures
  remain at depth seven. The check/mate/zugzwang/rule-fifty null guards and
  always-verified cutoff policy are unchanged.

The hash-locked endpoint input index is updated only for the reviewed move-set
additions. Final standard acceptance passes **11/11** records with maximum
measured reference loss **2 cp**; legacy acceptance passes **16/16**, maximum
20 cp. Original and final baseline acceptance reports are included too.
Optional "style improved" metrics remain false: this series claims retained
attacking character and stronger play, not a new sacrifice or attack-hit gain.

## Experiments that did not earn promotion

Eight independent candidates were screened on the same development book,
1,024 games each, 50 ms/move, Aggression 75. These are exploratory intervals,
**not adjusted for selecting among eight trials**. They must not be pooled as
independent confirmation.

| Candidate | Elo, paired-normal 95% interval | Disposition |
| --- | --- | --- |
| Two main-search check extensions below aggression 80 | -4.1 [-15.2, +7.0] | No demonstrated gain |
| Do not penalize unsearched quiets in history | -1.7 [-12.1, +8.7] | No demonstrated gain |
| No optional quiescence checks below aggression 80 | +4.8 [-6.7, +16.2] | Inconclusive; objective quiet-choice mismatch |
| Store quiet quiescence leaves | -2.4 [-12.9, +8.1] | No demonstrated gain |
| Weight quiescence capture history as depth one | -7.5 [-19.1, +4.1] | [0, 10] test accepts H0 |
| Store selective main-search bounds | +4.1 [-7.1, +15.3] | Inconclusive; failed old move/score checks |
| Optional quiescence checks only at the horizon below 80 | +11.9 [-0.0, +23.8] | Inconclusive; Greek-gift control reference loss 8 cp |
| PV reductions with one ply of relief | +12.2 [+0.6, +23.9] | Selected for separate confirmation |

Archived `*-screen.patch` files apply independently to the series base. The
quiet-check proposals are different from the already shipped one-check budget;
none of those additional changes is retained. The cache and history proposals
are not bundled with the accepted reduction rule.

### Rejected Aggression-75-only scope trial

After the first positive confirmation, a version restricted the PV reduction
change to exactly Aggression 75 to preserve endpoint behavior. Across **270**
fixed-node comparisons, its 75 searches matched the accepted candidate and its
other sampled profiles matched the base. Nevertheless, its timed results were:

| Scoped binary versus base | Games | Elo, paired-normal 95% interval | Verdict |
| --- | ---: | --- | --- |
| 50 ms | 3,072 | +1.2 [-5.9, +8.4] | continue |
| `1.0+0.01` | 2,000 | +7.1 [-1.9, +16.1] | continue |

That scope change was rejected, and the original immutable candidate was then
replicated on the second book. The scope trial reuses the already-played
confirmation book and is not a fresh holdout. Sampled tree equality does not
establish time-limited equality. These measurements do **not** establish whether
code layout, host scheduling, or sampling variation explains the discrepancy.
The failed replication is retained rather than hidden behind the positive runs.

## Provenance and validation

| Input | Identity |
| --- | --- |
| Series base | `7c362767b518091e20a934aca79cdf97eaaec7a8` |
| Accepted engine patch | `0cd9e0a91ad284e0e71364595272e0f157cec3ad` |
| Base executable SHA-256 | `272e3c056554956d84f32560299ccc8b86da17e6a47bdb2d9ec2a8706f55db2d` |
| Accepted executable SHA-256 | `85201a3ee2d2ad990a11de9c811bb12dcc8c4e4af1b41041ba87eb7fefabde6c` |
| Rejected scoped executable SHA-256 | `fa2fd6586cccc597473d2ddf4117c9b4d31c3445c05d3b90058ebf906177e325` |
| Locked cozy-chess revision | `5851b224ca58ef3330b9d41813fa76f391bb60cf` |

Builds use Rust/Cargo 1.88.0, `--release --locked`, fat LTO and one codegen unit.
Python is 3.12.3. The Linux x86_64 host reports 96 logical CPUs. Each engine has
one search thread and 16 MiB hash; screens use concurrency 20 and later matches
24. Matches, builds and checks overlap on a shared host, not a dedicated timing
machine. Manifests retain original paths and labels; the final rebuilt executable
is byte-identical to the accepted binary used throughout its matches.

Development uses all 512 positions in the
[evaluation pilot's development book](data/evaluation-refit-pilot/development.epd).
The initial confirmation uses all 1,536 positions in its
[reserved book](data/evaluation-refit-pilot/confirmation.epd); the clock uses the
first 1,000. Those parent seeds are separate from that pilot's development
parents; the book was unplayed by candidate engines before this round's initial
confirmation. They are still descendants of the repository's curated parents,
not an unrelated external database. Replication uses all 2,048 positions of
[the older selective-search book](data/selective-search-confirmation.epd), which
has extensive historical use. Book hashes and original commands are archived.

Observed validation on the committed engine:

- **283 default Rust tests** and **299 tuning-feature tests** pass.
- Formatting and Clippy with warnings denied pass; release rebuild matches the
  accepted executable hash.
- **50 Python tests** covering acceptance contracts/measurement and match
  statistics pass. The full unrelated Python suite was not rerun.
- Revised endpoint expected-move and safety gates, legacy and standard root-loss
  gates, and unchanged sacrifice gates pass. Original failures are archived.
- Documentation audit verifies every indexed artifact, all **13 complete PGNs
  (22,432 games)**, binary/book/manifest bindings, paired statistics, W/D/L,
  forcing rates, fixture locks and recorded test totals.

Unmeasured: tournament or other long controls, external opponents/books,
multithreaded strength, Elo at other aggression settings, and current 75-versus-0
personality cost. The scope trial illustrates why sampled tree/depth evidence
cannot substitute for matches. The shared-host measurements and related opening
families also limit how widely the roughly 13–16 Elo fixed-time gain generalizes.

## Reproduction and archive audit

Rebuild the named base and accepted engine revisions separately with the locked
release profile. Keep immutable executable copies. For the initial confirmation:

```sh
cargo build --release --locked --bin jakgro --bin selfplay
python3 tools/run_sprt.py \
  --engine target/release/jakgro --baseline-engine /path/to/base/jakgro \
  --runner target/release/selfplay \
  --candidate-aggression 75 --baseline-aggression 75 \
  --games 3072 --movetime-ms 50 --concurrency 24 --hash 16 \
  --elo0 0 --elo1 10 \
  --openings docs/tuning/data/evaluation-refit-pilot/confirmation.epd \
  --pgn /path/to/scratch/pv-lmr-confirm.pgn
```

For the clocked channel use `--games 2000 --time-control 1.0+0.01` instead of the
fixed-movetime arguments. For replication use `--games 4096` and
`docs/tuning/data/selective-search-confirmation.epd`. Timed games will not replay
bit for bit; the archived game/statistical audit is deterministic.

The [artifact index](data/aggression75-pv-lmr/sha256.json) covers compressed PGNs,
unmodified manifests/arbiter results, statistical and style summaries, declared
protocols, accepted/rejected experiment patches, before/after fixtures, probes
and validation logs. No executables or build products are included.

Audit without replaying games, from this source revision:

```sh
python3 docs/tuning/data/aggression75-pv-lmr/helpers/audit.py \
  --scratch /path/to/scratch/pv-lmr-audit
```

Optionally supply `--engine /path/to/rebuilt/accepted/jakgro` and
`--baseline /path/to/rebuilt/base/jakgro` to verify local binary hashes as well.
The helper decompresses into scratch, checks hashes and sequential reversed-color
openings, reparses every PGN, and recomputes both statistical reports. Archived
probe helpers document the original investigations; `probe-pv-lmr.py` expects
`JAKKOO_TEMP/strength-base` and `strength-pv-lmr`, while the rejected-scope helper
also expects `strength-pv75`. They are not required for the archive-only audit.
