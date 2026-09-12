# PGO and root-verification experiments

## Verdict: a faster optional build, and a separate soundness fix

Two independent experiments were retained:

- **Profile-guided optimization (PGO)** improves measured throughput by **10.7%**
  with identical fixed-depth search trees. Its 3,072-game confirmation at
  Aggression 75 and 50 ms/move estimates **+11.1 Elo [+4.1, +18.1]** using a
  paired-normal 95% interval; the final [0, 10] likelihood test accepts H1.
  Its clocked comparison remains inconclusive.
- **Root re-anchoring** fixes reproduced cases where completed verification finds
  a stronger move or a winning mate, but personality selection still prefers an
  ineligible weaker move. Its matches **do not demonstrate an Elo gain**. It is
  retained as a selection-soundness fix, not advertised as a strength increase.

**The experiments use different, explicitly recorded binary pairs.** The PGO
matches use the engine before the root fix on both sides. The root matches use
ordinary release builds, with no PGO on either side. No combined PGO/root gain
was measured; these estimates must not be added together or to earlier series.
Ordinary release builds remain unchanged, and Aggression still defaults to 75.

## Match results

All matches use Aggression 75 on both sides, one search thread per engine,
16 MiB hash, sequential reversed-color opening pairs and concurrency 20.
Every run completed without recorded engine faults. All caps were declared
before their respective results; the likelihood ratio was evaluated at the
cap, not used to stop games early.

| Comparison | Limit | Games | W / D / L | Elo, paired-normal 95% interval | Final LLR / decision |
| --- | --- | ---: | ---: | --- | --- |
| PGO screen | 50 ms/move | 1,024 | 414 / 220 / 390 | +8.1 [-3.1, +19.4] | 0.950 / continue |
| PGO confirmation | 50 ms/move | 3,072 | 1,208 / 754 / 1,110 | **+11.1 [+4.1, +18.1]** | 4.744 / H1 |
| PGO clock | `1.0+0.01` | 2,000 | 787 / 441 / 772 | +2.6 [-5.9, +11.1] | -1.280 / continue |
| Root-fix screen | 50 ms/move | 1,024 | 412 / 213 / 399 | +4.4 [-6.0, +14.8] | -0.208 / continue |
| Root-fix clock | `1.0+0.01` | 2,000 | 788 / 421 / 791 | -0.5 [-8.3, +7.3] | -3.480 / H0 |

The root clock's H0 verdict rejects the configured +10 Elo alternative; it does
not establish that the true change is exactly zero or negative. Neither root
interval excludes zero. The PGO clock likewise provides no confirmed gain.

The conservative Hoeffding Elo intervals retained by `analyze_match.py` are
[-33.7, +50.2], **[-13.0, +35.3]**, and [-27.3, +32.5] for the PGO screen,
confirmation and clock. The root intervals are [-37.4, +46.4] and [-30.4, +29.4].
All cross zero. The archive includes both methods rather than presenting the
narrower normal interval as the only uncertainty estimate.

These are short-control, same-engine-family comparisons on a shared host, not an
absolute rating, independent tournament validation, or a hardware-independent
Elo guarantee.

## PGO: what was built and measured

The opt-in [build helper](../pgo-builds.md) creates separate baseline,
instrumented and optimized build directories under a fresh output directory.
It does not change Cargo configuration or overwrite `target/release/jakgro`.
It records source, training-suite, tool, profile and executable hashes; failed
training, missing profiles, changed inputs or differing validation results
prevent publication of a validated PGO executable.

This run used:

- Rust/Cargo 1.88.0 and the matching LLVM 20.1.5 Rust tools;
- locked release builds with fat LTO, one codegen unit and an explicit
  `x86_64-unknown-linux-gnu` target;
- portable target CPU defaults, **not** `target-cpu=native`;
- 48 curated openings plus eight supplemental tactical/endgame positions;
- 250,000 nodes per position at Aggression 0, 75 and 100: **168 training searches**;
- one thread, 16 MiB hash, a new game for each training search;
- a separate ten-position validation suite at 100,000 nodes for those profiles:
  **30 baseline/PGO pairs and 238 completed iterations**, all identical in scores,
  depths, nodes, PVs, best moves and personality counters.

Training and validation starting records are disjoint after ignoring FEN move
counters. This is not a claim of unrelated structures, effective-en-passant
canonical separation, or absence of historical use. The profile is workload
training for compiler optimization, not a newly fitted chess evaluator.

On `tests/data/search-performance.epd`, depth eight and seven alternating samples
with 500 ms probes, both binaries search **1,115,382 total fixed-depth nodes**.
All ten trees are identical and repeatable. The geometric fixed-node NPS gain is
**10.719677%**, with **+0.1 completed ply** on average in timed probes. The safety
measurement briefly overlapped this run; PGO matches had not started yet. This is
one shared-host throughput measurement, not a dedicated-machine benchmark.

Raw profiles and the merged profile are compressed in the archive. The audit
checks their original byte hashes and can remerge them byte-identically with the
recorded LLVM tool. Instrumented and optimized binaries themselves are not
included. No CPU-specialized build, alternative training schedule or combined
PGO/root model was selected from these match results.

### How to use the tooling

For a new build of the current source:

```sh
rustup component add llvm-tools-preview
python3 tools/build_pgo.py --output-dir artifacts/pgo-run-1
```

Use a new output directory for each run. A successful run publishes
`jakgro-pgo`, `jakgro-baseline`, the profiles and `manifest.json` there. The helper
also accepts `--llvm-profdata /path/to/matching/llvm-profdata`; it never installs
tools itself. See the build guide for limits, overrides and portability cautions.

**A new run on the current source includes the root fix and is not the binary
pair matched in this report.** Its automatic equivalence validation still runs,
but the +11.1 Elo estimate is evidence for the archived pre-root-fix PGO pair,
not a measured combined result.

To repeat that isolated experiment, use source revision
`437283cb40615984809e4d69d090542ad5c94e4b`, where the PGO helper exists but root
selection is unchanged. The recorded run used the default training settings and
an explicitly supplied copy of the exact Rust LLVM tool. Different compiler,
linker, paths or training environments may change executable hashes.

## Root verification: what was fixed

Previously `choose_styled_candidate` used the original conventional result as
the score reference even after an alternative's completed full-window
verification found something stronger. It also began ranking from that original
candidate without rechecking its eligibility against any newly found strength.
Two synthetic selector regressions fail on the base:

1. A same-depth +100 cp alternative can lose to a high-interest +60 cp move while
   the old reference remains +50, exceeding the intended margin from the newly
   established best result.
2. A verified winning mate can lose to the original centipawn-scored move solely
   because the latter has higher personality interest.

The patch keeps the score at the conventional root depth separately from any
extra-ply investment verification. It establishes one reference over the whole
retained candidate set before ranking:

- for centipawn results, use the lower of same-depth and latest scores, so a deeper
  improvement does not overprice unextended alternatives and deterioration does
  not leave an unsupported high reference;
- a completed nonnegative result establishes at least a zero floor;
- when either result is mate-valued, the latest completed conclusion supersedes
  the earlier one;
- start with a candidate whose final score meets the reference, then apply the
  existing margins and interest pricing to every candidate, including the old
  conventional move.

This is a conservative policy for asymmetric verification depths, not a claim
that differently searched centipawn values are interchangeable. "Same depth"
means the requested search depth, not identical selective trees. Mate sign and
distance use the existing signed score ordering. Tie-breaking is independent of
candidate order and preserves the existing move-key rule.

The patch does **not** change aspiration centering, conventional root TT storage,
probing/verification budgets, interruption behavior, sacrifice requirements,
attacking bonuses, evaluation weights, check allowances or clock management.
Eight new tests cover the reproduced failures, depth provenance, mate ordering,
nonnegative floors, repricing, retained bounded attacking choices and permutations.
The old sacrifice and draw controls retain their assertions through a shared
test candidate constructor. No fixture or acceptance cap changes in this series.

On the frozen ten-position efficiency suite the root-only candidate and base
have identical depth-eight trees. Its +1.1% NPS and +0.1-ply diagnostic readings
are not interpreted as an additional performance or strength improvement.
The matches remain near neutral. The reason to keep the patch is the demonstrated
root-choice defect, not the positive point estimate in its short screen.

## Attacking personality and safety

| Comparison | Forcing-move rate retention | Check-rate retention |
| --- | ---: | ---: |
| PGO screen | 102.2% | 105.1% |
| PGO confirmation | **99.8%** | 98.2% |
| PGO clock | 100.1% | 100.4% |
| Root-fix screen | 99.1% | 96.2% |
| Root-fix clock | 99.3% | 98.9% |

These are SAN-derived descriptive rates, not independent attack-quality or
sacrifice-soundness labels. Both candidates pass the unchanged standard
acceptance, legacy root-loss, endpoint expected-move and sacrifice-control checks.
The default retains the `e5c6` investment, `c1g5` open-king attack and `d2e2`
queen-trade avoidance. No previously failed fixture was repinned in this round.
Optional "sacrifice improved" and "standard attacks improved" metrics remain
false: neither experiment is claimed to add new reviewed attacking choices.

## Provenance and limitations

| Artifact | Revision or SHA-256 |
| --- | --- |
| Series base | `20485cffbb55e8d0ee1249eedc8db5dea3ff302b` |
| PGO tooling patch / PGO playing-source revision | `437283cb40615984809e4d69d090542ad5c94e4b` |
| Root-selection patch | `fa9b8c78ab9e17e45ab8d02a429ac00071dcf1d5` |
| PGO pair: baseline | `2de6c272f330105767aa418c30c589e51a6d4f5c8ad29bab8cba4b7bbe81194d` |
| PGO pair: optimized | `82d0563efbc2f73e0616a58ee401545114f444c9604825c01773ddfc370b2fa3` |
| Root pair: baseline | `7328a3b15ca7cdd3214c1c614a933bff760d5d34f296d4c0fca95a7c67cdd471` |
| Root pair: candidate | `f21ade04a57d98101adc1166e3b6dec04a577c34036d7c60f8ca5e629706a39a` |
| Locked cozy-chess revision | `5851b224ca58ef3330b9d41813fa76f391bb60cf` |

The PGO baseline uses the helper's explicit host target and isolated build
path; it is not byte-identical to the ordinary release baseline used for root
selection. Both identities are retained and must not be substituted for one
another in statistical attribution. The PGO manifest's base revision label
predates the tooling commit; its source hashes include the subsequently
committed helper. Reversing the archived root patch reconstructs the exact
pre-root-fix source hash, which the audit verifies without Git history.

Screens use all 512 positions of
`data/evaluation-refit-pilot/development.epd`; confirmations use all 1,536 positions
of `data/evaluation-refit-pilot/confirmation.epd`, and clocks its first 1,000.
These books have already been used in earlier engine work and, in this round,
for the root-only tests. They are not newly unseen external opening families.
The PGO three-match protocol was fixed before any PGO game result; its channels
ran concurrently rather than selecting a profile based on the short screen.

The Linux x86_64 host reports 96 logical CPUs. PGO matches overlap one another;
root matches overlapped their validation jobs. These are shared-host timings.
Neither experiment was measured against external engines, at tournament or
200 ms controls, with multiple search threads, or for Elo at other aggression
settings. Current 75-versus-0 personality cost is unmeasured. Fixed-node equality
on a finite suite is not a proof of all-position equivalence or of equal timed
play. No fresh sampling-profile hotspot report, incremental evaluation rewrite,
NNUE, evaluation refit or CPU-native benchmark is claimed here.

## Validation and archive audit

Observed checks:

- **291 default Rust tests** and **307 tuning-feature tests** pass for the root fix.
- Formatting, Clippy with warnings denied and locked release rebuilding pass;
  the rebuilt root executable matches the immutable tested candidate.
- **86 relevant Python tests and 29 subtests** pass, covering PGO construction,
  UCI measurement, acceptance contracts and match statistics.
- The full default PGO build completes all three build stages, training and
  profile merge; all 30 validation pairs and 238 iterations agree.
- Unchanged hash-locked acceptance inputs validate; both candidates pass the
  required safety, style and sacrifice checks. The original two failing root
  regression results are retained as well.

The [artifact index](data/pgo-root-verification/sha256.json) covers all five complete
compressed PGNs (**9,120 games**), original manifests and arbiter reports, both
statistical analyses, protocols, throughput/safety reports, source and tool
hashes, raw/merged profiles, build logs and test output. No executable or Cargo
build directory is included. Historical reports are not modified.

Audit the retained evidence without playing new games:

```sh
python3 docs/tuning/data/pgo-root-verification/helpers/audit.py \
  --scratch /path/to/scratch/pgo-root-audit
```

The audit checks the inventory and hashes, reconstructs the PGO source from the
root patch, validates workload order and fixed-node equivalence, checks separate
binary-pair attribution and opening order, and recomputes every match's paired
statistics, W/D/L and forcing/check rates. It also verifies fixture identities and
recorded test totals. Supply `--llvm-profdata` with the exact recorded tool to
remerge raw profiles byte-identically; optional `--root-engine`, `--root-baseline`,
`--pgo-engine` and `--pgo-baseline` arguments verify local executable hashes.

For the isolated PGO confirmation, after rebuilding the tooling revision:

```sh
python3 tools/run_sprt.py \
  --engine /path/to/pgo-run/jakgro-pgo \
  --baseline-engine /path/to/pgo-run/jakgro-baseline \
  --runner target/release/selfplay \
  --candidate-aggression 75 --baseline-aggression 75 \
  --games 3072 --movetime-ms 50 --concurrency 20 --hash 16 \
  --elo0 0 --elo1 10 \
  --openings docs/tuning/data/evaluation-refit-pilot/confirmation.epd \
  --pgn /path/to/scratch/pgo-confirm.pgn
```

Build the runner with `cargo build --release --locked --bin selfplay`. For the
clock channel replace the game/limit options with
`--games 2000 --time-control 1.0+0.01`. Root-only reproduction instead uses ordinary
release builds of the named base and root revisions, with 1,024 games on the
development book at 50 ms and 2,000 clocked games on the confirmation book.
Timed games are not bitwise reproducible; auditing the archived game records is.
