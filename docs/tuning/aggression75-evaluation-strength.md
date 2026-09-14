# Aggression-75 evaluation strength trials

## Verdict: no demonstrated Elo gain

This round tried eight small evaluation fits, matched two mobility candidates,
and tested two exact objective-evaluation caches. **It did not find the
substantial playing-strength improvement sought.** No fitted weight or mobility
representation was retained. The final cache is an experimental checkpoint,
not a proven optimization or a deployment recommendation.

All three 1,024-game screens have paired-normal 95% intervals crossing zero.
The retained cache measures **−1.0 Elo [−11.7, +9.7]** at 50 ms/move and
**−0.06% throughput** on the sampled performance suite. There is no confirmation
match, clocked result, or evidence of a large gain hidden behind these numbers.
For deployment based on demonstrated strength, prefer the original baseline.

Default Aggression remains **75**. Evaluation weights, root attack preferences,
verified-sacrifice requirements, search settings and fixture expectations are
unchanged by the retained cache. It passes the existing personality and safety
checks, but passing those checks does not establish stronger play.

## Comparisons and results

Every match compares one immutable candidate with the same original baseline,
`9fda9ac72e63564e9f164c612376f28d0b549216`. Both sides use Aggression 75,
one search thread, 16 MiB hash and 50 ms/move. Each match plays all 512 openings
in reversed-color pairs with concurrency 16. There are **3,072 complete games**
in the archive and no recorded engine faults.

| Candidate versus original base | W / D / L | Elo, paired-normal 95% interval | Final [0,10] LLR | Decision |
| --- | --- | --- | ---: | --- |
| Pawn-safe mobility, outcome fit | 433 / 179 / 412 | +7.1 [−5.8, +20.1] | 0.490 | continue |
| Original mobility representation, outcome refit | 415 / 210 / 399 | +5.4 [−6.6, +17.5] | 0.114 | continue |
| Hash-prefiltered exact evaluation cache | 410 / 201 / 413 | **−1.0 [−11.7, +9.7]** | −2.029 | continue |

These are exploratory screens, not three independent confirmations. The
`run_sprt.py` harness plays the complete declared cap and then calculates its
paired-normal likelihood statistic; it did not stop these games sequentially.
Exit zero means clean completion, not acceptance of a strength hypothesis.
Do not pool the games across different candidates or add their Elo estimates.

The conservative pair-aware Hoeffding intervals are [−34.7, +49.2],
[−36.4, +47.4], and [−42.9, +40.9], respectively. They also all cross zero.

### Opening and timing limitations

All screens use the historical evaluation-pilot `development.epd`: 512 unique
endpoints from eight French, Caro-Kann, Pirc, Modern and Owen parent seeds.
Those families were separate from the pilot's training families, but the book
has been reused by previous experiments. It is **not a fresh holdout**. Pairing
controls color, not dependence among descendants of the same opening family.
The reported intervals do not adjust for that family clustering or selection
among multiple candidates.

The two mobility matches overlap in time. Some builds and validation jobs also
overlap games on this shared 96-logical-CPU host. The cache benchmarks were run
without match jobs, though the first benchmark overlaps short fixture checks.
No absolute rating, external-opponent, tournament-control, multithreaded or
PGO-combined strength claim follows from these measurements.

## Evaluation fits: all results, including those not matched

Search already uses the shared objective evaluator at Aggression 75. Changing
the static style coefficients would not directly change the ordinary negamax
or quiescence evaluation. This round therefore left aggression and root policy
alone and investigated objective mobility, king danger/safe checks and threats.

The existing pilot corpus supplies **140,541 training positions** and
**17,377 development-loss positions**. It contains White-relative game outcomes
and search scores; the earlier corpus was generated at Aggression 0 and
200 ms/move, not at the default profile. Only training samples feed these fits.
The compressed corpus is referenced by SHA-256 rather than duplicated.

Each fit uses the existing 611 MG/EG feature pairs, 100 full-batch Adam epochs,
rate 0.25, L2 `1e-6` toward the compiled baseline weights, minimum 2,000 nonzero
observations and `--holdout 0`. The complement of the named free blocks is held:

- **Mobility:** N/B/R/Q mobility curves and `UNSAFE_MOBILITY_BY_PIECE`.
- **Safety:** `KING_DANGER_BY_BUCKET` and `SAFE_CHECK_BY_PIECE`.
- **Threats:** pawn-on-minor, hanging, lower-value-attacker and pawn-push threats.

Material and piece-square tables remain fixed. The second mobility
representation indexes curves by destinations outside enemy pawn control;
raw attack maps and diagnostic mobility totals remain unchanged. Its residual
unsafe-mobility weights were refitted, **not constrained to be nonpositive**.
It is not a pin-aware mobility implementation or legal-move enumeration.

| Representation / free family | Outcome lambda | Changed pairs | Largest coordinate change | Emitted development outcome MSE | Matched? |
| --- | ---: | ---: | ---: | ---: | --- |
| Baseline | — | — | — | 0.064476326 | reference |
| Original / mobility | 1.0 | 29 | 5 cp | 0.064441048 | yes |
| Original / mobility | 0.5 | 19 | 2 cp | 0.064470882 | no |
| Original / safety | 1.0 | 8 | 2 cp | 0.064487301 | no |
| Original / safety | 0.5 | 8 | 1 cp | 0.064484440 | no |
| Original / threats | 1.0 | 3 | 1 cp | 0.064476187 | no |
| Original / threats | 0.5 | 2 | 1 cp | 0.064477925 | no |
| Pawn-safe / mobility | 1.0 | 32 | 7 cp | 0.064684983 | yes |
| Pawn-safe / mobility | 0.5 | 17 | 5 cp | 0.064694905 | no |

Lambda 0.5 blends the outcome with the recorded search-score probability.
The integer-model scorer uses **K = 0.8806824810924139**, fitted once to baseline
training scores and frozen across candidates and development scoring. Optimizer
K is approximately 0.8792 for original features and 0.8767 for pawn-safe features.
Printed optimizer loss precedes rounding and is not the emitted-model MSE.
With holdout zero, a printed held-out loss of zero means **no holdout samples**.

The small safety/threat fits offered little development-loss change, so they
were not built or matched. The pawn-safe outcome fit was screened despite its
worse development MSE to test the representation in play. Neither mobility
screen established a gain; the other six fits have **no strength measurement**.
The initial protocol, all fit commands, logs, vectors, held-block lists, losses,
and the two installed-vector comparisons are retained. No coefficients were
selected to restore a failed historical move choice.

## The retained exact cache

Commit `575c534727bfecb7b8f4be7bb0161a9ae05f392b` adds a per-thread,
direct-mapped memo around the shared objective static scorer. A 64-bit board
hash first rejects misses; hits then compare all piece bitboards, White's color
bitboard, side to move and complete castling-right fields. The score does not
depend on clocks, repetition or en passant; en passant may still alter the hash
prefilter and cause a harmless miss. Hash collisions cannot authorize an
incorrect score on their own.

The memo stores **static scores only**. It does not cache terminal/draw results,
change the search transposition table, or influence replacement and cutoffs.
The final 8,192-slot table uses 64-byte entries and eight-byte hash tags:
**576 KiB per evaluating thread**, in addition to existing caches. Lazy
allocation and the extra lookup are real costs. No hit-rate instrumentation
was collected, so this round does not explain the neutral runtime result by
an observed hit ratio.

The first version compared exact entries without the hash prefilter and used
512 KiB. It was benchmarked, not matched. Both versions preserve the sampled
fixed-depth best move, score, depth and node count, including repeated runs.
That is narrower evidence than comparing every internal search event or PV.

| Cache versus original base | Identical sampled trees | Fixed-depth nodes, each side | Geometric NPS change | Mean timed depth change |
| --- | ---: | ---: | ---: | ---: |
| Untagged exact entries | 10/10 | 1,115,382 | −0.35% | 0.0 ply |
| Hash-prefiltered entries | 10/10 | 1,115,382 | **−0.06%** | −0.1 ply |

The performance suite uses depth eight, seven alternating fixed-node samples
and 500 ms timed probes per position. The generic efficiency gate passes because
its NPS floor is −2%; **that is not a measured speedup**. No timing confidence
interval was estimated. Only the tagged variant reached a match, and its result
was inconclusive. Retaining this experimental checkpoint does not satisfy the
initial strength-promotion condition; it is not recommended as an upgrade.

## Personality and failed checks

| Candidate | Endpoint expected moves | Sacrifice-suite expected moves | Standard acceptance | Forcing-rate retention | Check-rate retention |
| --- | ---: | ---: | ---: | ---: | ---: |
| Original baseline | 32/32 | 8/8 | 11/11 | reference | reference |
| Pawn-safe mobility fit | 28/32 | 5/8 | 7/11 | 100.8% | 104.6% |
| Original-representation mobility fit | 31/32 | 8/8 | 9/11 | 101.7% | 103.4% |
| Tagged cache | **32/32** | **8/8** | **11/11** | **96.2%** | **91.3%** |

SAN-derived rates describe complete-game activity, not attack quality or
sacrifice soundness. The cache retains the standard knight investment,
open-king attack and queen-trade avoidance under the fixed-node fixtures.
Its lower match check rate is reported rather than described as identical play.
The code's preferences are unchanged, but timed searches can select differently.

The pawn-safe fit changes the knight investment to `d2d4` at both 75 and 100.
Its unsupported-Greek-gift fixture chooses quiet alternatives, not the forbidden
bishop sacrifice, but still fails the recorded expected-move checks. Both
mobility variants fail the short storm record because profile 0 changes, even
though `b2b4` is an allowed profile-75 move. The weight-only fit also exceeds
the Greek-gift record's objective-reference loss cap: **17 cp versus 1 cp**.
The pawn-safe fit passes all root-loss caps but fails expected choices.
No fixture was repinned and no safety/loss cap was relaxed to admit a candidate.

Two validation mistakes/findings are explicitly retained:

1. An initial baseline call requested `0,75,100` on `personality.epd`, which has
   only endpoint expectations. Its 16 missing-profile mismatches are not engine
   regressions. Correct endpoint and standard-profile invocations are archived
   separately; the original failed output is not replaced.
2. The cache's first wider playout test used the styled diagnostic trace as its
   oracle and found a pre-existing one-centipawn difference: objective 172,
   trace 171, at `1rb5/5p2/1pnkp3/p1pN2Pr/2P3P1/6KN/PBqP2BP/1R5R b - - 1 26`.
   The final memo test compares directly with the **uncached objective path**,
   not a tolerance-adjusted trace. Its 4,096 playout steps, repeated evaluations
   and available null moves pass. The styled/objective discrepancy remains
   unresolved and was not folded into a score-preserving cache patch.

## Validation, provenance and reproduction

Final observed validation: **296/296 default tests**, **312/312 tuning-feature
tests**, `cargo fmt --check`, warnings-denied all-target/all-feature Clippy,
and a locked release rebuild. The rebuilt executable equals the immutable
matched cache binary. Tests include forced slot/hash collisions, every cached
input family, repetition hits, clock/EP metadata and uncached-score comparison.
No engine or fixture file changes in this documentation patch.

The documentation archive passes its offline audit over all 3,072 games, seven
archive-helper tests and 118 measurement-tool tests. Reconstructed base and cache
sources both build successfully outside the checkout. Their executable hashes
differ from the original match binaries; the attempted byte comparison exits 1,
and that output is retained. This is source reproduction, not a claim of
bit-identical builds across directories. No new match was played for this check.

Builds use Rust/Cargo 1.88.0, fat LTO, one codegen unit, portable CPU defaults,
and cozy-chess revision `5851b224ca58ef3330b9d41813fa76f391bb60cf`. There is no PGO
or CPU-native comparison. Original match manifests retain null candidate
revisions and their disposable paths; `provenance.json` supplies separate
source-stage and binary-hash bindings rather than rewriting historical records.
The final candidate SHA-256 is
`34a3a33b08b1d26d8eda55280fdbb07e796df9f9a27b18c600d69d771a533481`;
the original base is
`c78cbaa1387bd59ffd43d289f359106a2ae3b6d86023d570055cf5c1eb380516`.

The [archive](data/aggression75-evaluation-strength/sha256.json) includes all
three compressed PGNs, manifests, arbiter output, protocols, independent
before/after input hashes, statistical analyses, failed gates, eight fits,
cache benchmarks, source-stage patches, validation logs and frozen helper/fixture
inputs. Binaries, build products and expanded training files are not committed.

Audit without playing new games (choose scratch outside the checkout):

```sh
ART="$PWD/docs/tuning/data/aggression75-evaluation-strength"
R="$(mktemp -d)"
PYTHONDONTWRITEBYTECODE=1 python3 "$ART/helpers/audit.py" --scratch "$R/audit"
PYTHONDONTWRITEBYTECODE=1 python3 "$ART/helpers/test_archive.py"
```

The audit verifies the archive index and source reconstruction, decompresses
PGNs, checks opening order/color pairs, rehashes the recorded inputs, recomputes
W/D/L, paired LLR/intervals, Hoeffding intervals, style rates, acceptance outcomes
and cache efficiency summaries. It checks fit deltas/held blocks and the original
failure/test evidence. It does **not** replay move legality, rerun searches, or
recompute fitted-model MSE. Optional `--binary ROLE=PATH` verifies local executable
hashes. Helper tests also check that a changed arbiter result is rejected.

Reconstruct and build any recorded stage without modifying this checkout:

```sh
python3 "$ART/helpers/prepare_stage.py" --stage base --out "$R/base"
python3 "$ART/helpers/prepare_stage.py" --stage memo-tagged --out "$R/memo"
(cd "$R/base" && cargo build --release --locked --bin jakgro --bin selfplay)
(cd "$R/memo" && cargo build --release --locked --bin jakgro)
python3 tools/run_sprt.py \
  --engine "$R/memo/target/release/jakgro" \
  --baseline-engine "$R/base/target/release/jakgro" \
  --runner "$R/base/target/release/selfplay" \
  --candidate-aggression 75 --baseline-aggression 75 \
  --games 1024 --movetime-ms 50 --concurrency 16 --hash 16 \
  --elo0 0 --elo1 10 \
  --openings "$ART/inputs/docs/tuning/data/evaluation-refit-pilot/development.epd" \
  --pgn "$R/repeated-screen.pgn"
```

Stages `raw-mobility-1`, `safe-mobility-1`, `safe-prior` and `memo-8192` reconstruct
the rejected weight candidate, rejected representation candidate, representation
with original weights, and first cache, respectively. The last snapshot includes
a subsequent test-only oracle correction; its production implementation is the
measured one. Rebuilds can change hashes with toolchain or path differences;
timed game outcomes are not bitwise reproducible.

To repeat an original-representation fit and its emitted development MSE:

```sh
(cd "$R/base" && python3 \
  docs/tuning/data/evaluation-refit-pilot/helpers/build-refit-helpers.py \
  --out "$R/fit-bin")
gzip -dc docs/tuning/data/evaluation-refit-pilot/training.filtered.txt.gz > "$R/train.txt"
gzip -dc docs/tuning/data/evaluation-refit-pilot/development.filtered.txt.gz > "$R/dev.txt"
"$R/fit-bin/refit-tune-base" fit --positions "$R/train.txt" --out "$R/mobility.fit.rs" \
  --holdout 0 --lambda 1 --epochs 100 --rate 0.25 --l2 1e-6 \
  --min-observations 2000 --hold "$(cat "$ART/data/mobility.hold")"
python3 docs/tuning/data/evaluation-refit-pilot/helpers/refit-convert.py \
  "$R/mobility.fit.rs" --layout "$ART/data/layout.tsv" --out "$R/mobility.tsv"
"$R/fit-bin/refit-score" evaluate "$R/dev.txt" "$R/mobility.tsv" "$ART/data/calibration.k"
cmp "$R/mobility.tsv" "$ART/data/raw-mobility-1.tsv"
```

Use the safety/threats hold list or lambda 0.5 for the other small fits. For the
pawn-safe fits, build the fitter and scorer from `safe-prior`, not from a stage
that already contains fitted weights: the compiled weights are both the starting
point and regularization target. The original orchestration scripts and exact
commands are archived under `helpers/original` and in each fit record.
