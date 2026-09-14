# Pin, safe-check and pawn-race evaluation trials

## Verdict: more explicit evaluation knowledge, no demonstrated Elo gain

This series tested pin-aware mobility, post-move safe-check validation and a
conservative pawn-only race term at the default Aggression 75. The combined
engine estimates **+2.9 Elo [-3.9, +9.6]** over 4,096 games at 50 ms/move and
**-2.3 Elo [-11.8, +7.3]** over 2,000 games at `1.0+0.01`, using paired-normal
95% intervals. Both intervals cross zero. The clocked likelihood test accepts
H0 for the configured [0, 10] alternatives.

**These changes are not a confirmed strength upgrade.** The retained code adds
specific evaluation knowledge and passes the Rust regression suites, but the
combined result does not justify an Elo claim. For deployment chosen solely on
proven playing strength, this round supplies no reason to prefer it over the
original baseline. The pawn-race coefficient remains an experimental hand-set
weight, not a demonstrated improvement.

The attacking root policy and default aggression are unchanged. The combined
matches retain 100.4% and 101.0% of the baseline's forcing-move rates. That is a
style diagnostic, not evidence of better attacks. One non-forced, short-budget
pawn-storm fixture changes; its failed expected-move check is retained rather
than hidden by changing the fixture.

## Match results and attribution

Every match uses Aggression 75 on both sides, one search thread per engine,
16 MiB hash and concurrency 20. Openings are played in reversed-color pairs.
All nine runs completed without recorded engine faults. Caps were declared
before the corresponding results; the final likelihood ratios were evaluated
at those caps rather than used to stop games early.

Component comparisons are **incremental**: pins versus the original base,
safe checks versus pins, and races versus pins plus safe checks. Only the
combined rows compare all three changes with the original base. Do not add
component Elo estimates or pool these different binary pairs.

| Comparison | Limit | Games | W / D / L | Elo, paired-normal 95% interval | Final LLR / verdict |
| --- | --- | ---: | ---: | --- | --- |
| Pins vs base | 50 ms/move | 1,024 | 423 / 200 / 401 | +7.5 [-3.6, +18.6] | 0.771 / continue |
| Pins vs base | `1.0+0.01` | 2,000 | 801 / 426 / 773 | +4.9 [-4.1, +13.8] | -0.064 / continue |
| Rejected full safe checks vs pins | 50 ms/move | 1,024 | 398 / 202 / 424 | -8.8 [-20.7, +3.0] | -3.782 / H0 |
| Scoped safe checks vs pins | 50 ms/move | 1,024 | 424 / 200 / 400 | +8.1 [-2.9, +19.2] | 0.984 / continue |
| Scoped safe checks vs pins | `1.0+0.01` | 2,000 | 809 / 396 / 795 | +2.4 [-5.8, +10.7] | -1.450 / continue |
| Pawn races vs two-patch baseline | 50 ms/move | 1,024 | 404 / 213 / 407 | -1.0 [-10.8, +8.7] | -2.426 / continue |
| Pawn races vs two-patch baseline | `1.0+0.01` | 2,000 | 813 / 381 / 806 | +1.2 [-5.1, +7.5] | -3.650 / H0 |
| All three vs original base | 50 ms/move | 4,096 | 1,507 / 1,116 / 1,473 | **+2.9 [-3.9, +9.6]** | -1.778 / continue |
| All three vs original base | `1.0+0.01` | 2,000 | 748 / 491 / 761 | **-2.3 [-11.8, +7.3]** | -3.045 / H0 |

Every paired-normal interval crosses zero. An H0 verdict here rejects the
configured +10 Elo alternative; it does not prove the true effect is exactly
zero or negative. The archive also retains conservative Hoeffding intervals.
For the combined fixed-time and clocked matches those are [-18.0, +23.8] and
[-32.2, +27.6], respectively. All nine conservative intervals cross zero too.

These are short-control self-play measurements on a shared host, not absolute
ratings, tournament validation or comparisons with an external engine.

## What changed

### 1. Pin-aware objective mobility and direct threats

The old fused scan awarded a pinned knight its geometric mobility and could
count a queen threat from a knight that could not legally make the capture.
Two focused regressions reproduce those overcounts before the patch.

The evaluator now finds absolute pins for either color, independent of whose
turn it is. Mobility and direct-threat reach are restricted to the king/pinner
line. Knights lose unusable mobility; sliders and pawns keep moves along the
pin line, including captures of the pinner. The corresponding unsafe-mobility
counts and tuning-vector expansion use the same restriction.

Geometric attack maps remain separate and unchanged for king safety, king-danger
features and attacking-style calculations. A pinned piece still controls squares
against an enemy king. The set-wise pawn fast path remains available when no
pawn is pinned. This is still an evaluation approximation, not full legal move
generation: it does not turn every mobility count into a check-evasion count.

Tests cover mirrored knights, sliders and pawns, multiple blockers, mismatched
slider directions, preservation of geometric control, a differential playout
against the move generator's own-piece pin mask, and tuning-vector parity.
No existing evaluation weight was refitted for this patch.

### 2. Scoped safe-check validation

The original safe-check estimate intersects aggregate attack maps with checking
squares. It cannot fully account for vacating the checking origin, newly opened
capture rays or an existing check that the proposed move fails to evade.

The retained version validates **knight, bishop and rook** candidates against
the resulting occupancy. It verifies direct check and king safety, then probes
only possible immediate captures of the checking piece. A pinned enemy defender
cannot make an illegal capture, while a pinned friendly defender still controls
a square against king capture. A destination counts once per piece type even
when several pieces can reach it. No board cloning, full move generation or
static exchange search is used in the production evaluation path.

The **queen slot retains its original geometric approximation**. The existing
candidate prefilter is also retained, so some safe checks with newly uncovered
support or pinned defenders remain uncounted. This is a conservative filter of
the fitted feature, not an exhaustive list of safe legal checks or a claim that
a check wins material. Pawn checks, promotions, castling and discovered-only
checks remain outside the feature.

Tests use an independent legal-move/capture oracle, mirrored positions and
playouts, along with the unchanged older check-count fixtures and tuning-vector
round trips. Existing feature indices and safe-check weights are unchanged.

#### Rejected broader versions

- The full origin-aware N/B/R/Q implementation passed its legality tests but
  cost 7.6% measured throughput, broke the depth-nine null-search root-move
  equality fixture and made Aggression 100 accept the equal queen trade. Its
  1,024-game screen estimated -8.8 Elo and accepted H0. It was not promoted.
- Restricting the full implementation to the old candidate prefilter restored
  the null-search equality check but retained the queen-trade regression.
  Default tests finished 301/302 passing and tuning tests 319/320 passing.
  This variant was built and probed, not matched.
- A diagnostic variant treating check opportunities as latent rather than
  requiring immediate check evasion still chose the equal queen trade. It was
  not matched or promoted.
- A single declared calibration of only `SAFE_CHECK_BY_PIECE` used 140,541
  retained training positions with freshly extracted features, 200 epochs,
  rate 0.25, L2 `1e-5`, outcome labels and no internal holdout. The other 607
  of 611 feature slots were held. It emitted the same four integer weight
  pairs: `(23,-1), (15,15), (25,9), (45,15)`. No calibrated gain or independent
  held-out validation is claimed. The fit, protocol and log are retained.

The final N/B/R scope was selected after those failed experiments. Its positive
screen point estimate is not an independent discovery or proof of improvement.
The original failures and discarded patches are archived alongside the final
passing tests; no personality expectation was weakened to admit the change.

### 3. Conservative pawn-only race observation

The new `PAWN_RACE` feature contributes at most one White-relative runner,
with an experimental weight of **0 middlegame / 80 endgame centipawns**. It is
additive: it does not scale the entire ending, adjudicate a draw or declare a
forced win. Existing passer, king-distance and attacking-style weights remain
unchanged.

Eligibility is deliberately narrow:

- Only kings and pawns may remain, the position must not be in check, and no
  en-passant opportunity may exist.
- A candidate must be genuinely passed and its entire file ahead through the
  promotion square must be empty, including friendly blockers.
- The push schedule accounts for side to move and the initial double push.
  Each landing is checked against an optimistic, unobstructed route for the
  defending king, including its reply after promotion. Friendly king support
  and other protection are ignored, so supported runners can go unrewarded.
- Every opposing pawn supplies an optimistic earliest promotion time, even if
  blocked or catchable. A race within two plies is suppressed. Promotion checks
  and close competing promotions are left to search.
- Optimistic opposing pawn reach also suppresses running plans that could be
  interrupted by a capture, blockage or check on the stationary friendly king.

The tempo-sensitive result is computed outside the pawn/king structure cache.
The tuner's new scalar appends at index 611, growing the vector from 611 to 612
slots without moving existing indices. Old extracted vectors do not acquire
this information automatically and should be regenerated for future fitting.

Tests cover mirrored tempo and double-push boundaries, capture before the first
push, a blocked promotion square, own-pawn blockers, en passant, check,
competing promotions, cache separation and at-most-one-runner scoring. Selected
credited runs are independently checked against every legal defending-king
reply. These tests do not constitute an exhaustive pawn-ending solver. Richer
connected-passer coordination, outside-passer diversions and exact supported
king routes were not implemented in this trial.

## Search cost

The ten-position performance suite uses depth eight, five alternating samples
per component (seven for the combined engine), and 500 ms timed probes. It was
not used as a substitute for match results. Evaluation changes can alter trees,
so node counts and NPS describe different effects.

| Comparison | Fixed-depth node change | Geometric NPS change | Mean completed depth change |
| --- | ---: | ---: | ---: |
| Pins vs base | +7.36% | -3.11% | -0.2 ply |
| Rejected full safe checks vs pins | -4.05% | -7.56% | 0.0 ply |
| Scoped safe checks vs pins | +0.01% | -0.91% | -0.1 ply |
| Pawn races vs two-patch baseline | 0.00% | -0.91% | 0.0 ply |
| All three vs original base | **+7.37%** | **-3.99%** | **-0.3 ply** |

Combined depth-eight node totals are 1,277,131 versus 1,115,382. Only three of
the ten combined trees are identical to the original baseline; all are
repeatable. The race-only comparison has identical sampled trees. These
shared-host readings show **no speedup**. Several generic efficiency gates
report false because the changes do not meet their node-reduction or throughput
thresholds; those false verdicts are retained and recomputed by the audit.

## Personality, safety and the unchanged fixture mismatch

| Comparison | Forcing-rate retention | Check-rate retention |
| --- | ---: | ---: |
| Pins, screen / clock | 99.7% / 100.0% | 99.8% / 100.2% |
| Scoped safe checks, screen / clock | 99.1% / 99.3% | 97.5% / 97.5% |
| Pawn races, screen / clock | 101.6% / 100.9% | 103.8% / 102.2% |
| Combined, fixed time / clock | **100.4% / 101.0%** | **101.9% / 104.4%** |

These are SAN-derived descriptive rates, not independent attack-quality or
sacrifice-soundness labels. Endpoint style, sacrifice and legacy root-loss gates
pass for the retained variants. The default retains the `e5c6` investment,
`c1g5` open-king attack and `d2e2` queen-trade avoidance. Optional "sacrifice
improved" and "standard attacks improved" metrics remain false.

**Standard acceptance is 10/11, not an unqualified pass.** The original base
passes 11/11. At 20,000 nodes, `standard-opposite-castle-storm` changes from the
reviewed Aggression-75 alternatives (`b2b4`, `d2b3`, `f3e5`) to `f1e1` after the
pin patch, and remains so after the other two patches. That selected move has
zero measured objective-reference loss under the same candidate. This does not
certify its chess quality or erase the expected-move failure. The separate
400,000-node storm record still selects `b2b4`; all mandatory safety controls
and all root-loss caps pass unchanged.

No fixture was repinned, no loss cap increased and no existing personality
assertion relaxed. The recorded exception is consistent with allowing different
non-forced moves, but users requiring every historical standard-profile choice
should not treat this series as satisfying that requirement.

## Provenance and limits

| Stage | Revision | Executable SHA-256 |
| --- | --- | --- |
| Original base | `9fda9ac72e63564e9f164c612376f28d0b549216` | `8d874dad48a15ddc0c88ccbc430781e7aee960067a9d5c6bc6a2e7de17433f61` |
| Pins | `03c5d7e07c7be8896e9c9b12bb220064a624a64b` | `83f9cd4942891c6694dc8dcacf8c5a9f9096a78c3e6ac5b0550d8268206637b0` |
| Pins + scoped safe checks | `ede85b3dd1757477911cc6ae1f72a7fbf2176f55` | `fad263700afb397d01350cd17ff6c154139620cc387bd89ed1a53fae35332d1a` |
| All three | `042f1eeea55fc7bef46105ed9b61726b721e400c` | `f51cf037f5bdd45ed54a1e77e2333ae5b40b82e3af4f831b7a2d7d702d487799` |

Builds use Rust/Cargo 1.88.0, locked release dependencies, fat LTO, one codegen
unit and portable CPU defaults. No PGO or CPU-native builds are used. The locked
cozy-chess revision is `5851b224ca58ef3330b9d41813fa76f391bb60cf`; Python is 3.12.3.
The Linux x86_64 host reports 96 logical CPUs. Match channels and validation
jobs overlap on this shared host.

Component screens use the 512-position evaluation-pilot development book;
component clocks use the first 1,000 of its 1,536 confirmation positions. Combined
testing uses the 2,048-position selective-search book, all positions at 50 ms and
the first 1,000 for the clocked match. These books have substantial historical
use and related curated opening families. The combined book differs from this
round's component-test books, but is not a new external holdout. The names
"development" and "confirmation" do not remove those limitations.

Unmeasured: longer/tournament controls, external opponents, multithreaded Elo,
Elo at other aggression settings, current 75-versus-0 personality cost, and this
evaluation series combined with PGO. No cumulative Elo gain over prior releases
is claimed. The additive pawn-race rule and retained pin/safe-check changes
remain unproven as strength improvements despite their focused correctness tests.

## Validation and reproduction

Observed Rust suite totals are 297 default / 314 tuning-feature tests after pins,
302 / 320 after scoped safe checks, and **307 / 326** after pawn races. Formatting,
Clippy with warnings denied and locked release rebuilds pass; the final rebuilt
executable matches the immutable match candidate. The full Python helper suite
passes **118 tests and 41 subtests**. The earlier failing and prematurely stopped
runs remain in the archive with their actual counts.

The [artifact index](data/eval-pins-checks-races/sha256.json) covers nine complete
compressed PGNs (**16,192 games**), original manifests and arbiter reports,
paired and conservative statistical analyses, protocols, diagnostic reports,
failed gates, rejected patches, calibration inputs by hash, source-stage
bindings, fixture snapshots and validation logs. Executables, Cargo build
products and disposable training expansions are not committed. The existing
compressed training corpus is referenced by hash rather than duplicated.

Audit the retained evidence without playing new games:

```sh
python3 docs/tuning/data/eval-pins-checks-races/helpers/audit.py \
  --scratch /path/to/scratch/eval-audit
```

The audit reconstructs each earlier source stage by reversing the archived
patches in memory, verifies fixture and measurement-tool identities, checks
binary-pair attribution and opening order, and recomputes paired statistics,
W/D/L, forcing/check retention and efficiency metrics. It checks the original
failure evidence, unchanged calibration output and recorded test totals.
Optional repeated `--binary ROLE=PATH` arguments verify local executable hashes,
for example `--binary eval3-races=/path/to/final/jakgro`. Scratch must be outside
the archive; original manifests are retained unchanged.

To repeat the combined comparison, build the named base and final revisions
separately with `cargo build --release --locked --bin jakgro --bin selfplay`,
and retain immutable executable copies. Then run:

```sh
python3 tools/run_sprt.py \
  --engine /path/to/final/jakgro --baseline-engine /path/to/base/jakgro \
  --runner /path/to/selfplay \
  --candidate-aggression 75 --baseline-aggression 75 \
  --games 4096 --movetime-ms 50 --concurrency 20 --hash 16 \
  --elo0 0 --elo1 10 \
  --openings docs/tuning/data/selective-search-confirmation.epd \
  --pgn /path/to/scratch/combined.pgn
```

Use `--games 2000 --time-control 1.0+0.01` for the clocked channel. The component
protocols name their different baselines and books. Rebuilding can change binary
hashes with different compilers or paths, and timed games are not bitwise
reproducible. Auditing the retained game records is separate from replaying them.
