# Disjoint-corpus evaluation-tuning pilot

## Verdict

**No evaluation weights, search policy, personality parameters or fixtures are
changed by this record.** The four tested models remain research artifacts.
Their development matches were inconclusive and none passed the existing
attacking-choice checks unchanged. The accepted engine remains the deployment
recommendation.

The best development point estimate was +10.5 Elo, but its paired 95% interval
included zero and it no longer chose the reviewed default-profile knight
investment. That is a promising candidate for further study, **not a confirmed
gain**. The confirmation book was generated and audited for identity overlap,
but no candidate searches or games were run on it. No confirmation result was
used to choose or revise a fit.

The useful retained work is a reproducible new corpus, canonical overlap checks,
a scorer for the emitted integer model, four fitted weight sets, complete game
records, and the failed checks as well as the positive measurements.

## Development results

All match comparisons below are candidate versus frozen base at **Aggression
75**, 1,024 games, 512 color-reversed pairs, 50 ms/move, one thread and 16 MiB hash.
Every run completed without engine faults. All final [0, 10] LLR decisions were
`continue`; no development run reached H1.

| Model | Emitted development outcome MSE | Development Elo, paired normal 95% interval | Forcing-rate retention |
| --- | ---: | --- | ---: |
| Unchanged base | 0.064476326 | reference | reference |
| Conservative, outcome only | 0.064453666 | **+5.4 [-5.7, +16.5]** | 101.1% |
| Weaker prior, outcome only | 0.064088223 | **+3.4 [-8.7, +15.5]** | 99.6% |
| Hybrid outcome/search score | 0.064114438 | **+10.5 [-1.3, +22.4]** | 99.9% |
| Hybrid, material/placement held | 0.064458054 | **+7.1 [-4.7, +19.0]** | 102.0% |

These are exploratory, unadjusted development intervals after trying several
models on the same book. Do not combine them into a larger independent test or
present the best estimate as validation. Better outcome prediction did not, by
itself, establish better play.

Every development parent contributes 64 opening pairs. Leave-one-parent-out
point-estimate ranges were +3.1 to +9.7, -0.4 to +8.1, +5.8 to +12.0, and +3.5
to +10.9 Elo respectively. These are sensitivity diagnostics, not confidence
intervals. Eight parent seeds do not provide hundreds of independent opening
families.

## What was fitted

The existing evaluation has **611 feature slots, each with middlegame and
endgame weights: 1,222 optimizer coordinates**. No feature extraction, search
setting or root-style rule was altered. In particular, aggression 75 keeps its
one optional quiet check, three main-search check extensions and existing
root-risk guards.

All fits used the same immutable baseline fitter and began from the same
published weights:

- 200 full-batch Adam epochs, learning rate 0.25;
- minimum 2,000 nonzero observations per feature;
- regularization toward the published weights, not toward zero;
- `--holdout 0`, because the built-in every-tenth-position split is not an
  independent game-level validation set;
- the existing centering and middlegame-pawn anchor of 94 before emission.

The first two variants were declared before data generation: outcome-only
labels with L2 `1e-5` and `1e-6`. After their inconclusive screens, two additional
**development-only** trials were explicitly recorded before running them:

1. `--lambda 0.5 --l2 1e-6`, blending game outcomes with the recorded deeper
   search-score probability;
2. the same hybrid fit while holding `PAWN,KNIGHT,BISHOP,ROOK,QUEEN,KING`, the
   three per-pawn material terms and `BISHOP_PAWNS_ON_COLOUR` at the prior.

The named piece blocks include both material scalars and piece-square tables
where both exist. In this final constrained model the emitted material and
placement values were also checked to equal the base. The first three fits held
235 feature pairs for low support; the constrained fit held 430 pairs through
the union of low support and explicit holds. These counts are neither independent
game counts nor separate MG/EG support guarantees.

The conservative model changed 84 weight pairs, with a largest coordinate change
of 7 cp. The weaker-prior model changed 460 pairs, with a largest coordinate change
of 26 cp. A coordinate change is not a bound on position-score change: feature
counts can amplify it. Full emitted vectors and all model summaries are archived.

### Score orientation and loss reporting

The repository PGNs annotate the search score **before the move**, relative to
the mover. Extraction flips Black's score once, giving White-relative centipawns
alongside the White game outcome. No additional sign conversion was made.

The fit uses sigmoid mean squared error with

```text
p(score) = 1 / (1 + 10^(-K * score / 400))
label = lambda * game_outcome + (1 - lambda) * p(search_score)
```

The fitter's printed K was 0.8792. Its reported final loss precedes centering,
anchoring and integer rounding, and blended-label training loss is not comparable
with pure-outcome loss. Because its internal holdout was disabled, its printed
`0.000000 held out` values mean **no held-out samples**, not perfect validation.

The independent `refit-score` helper instead scores the **emitted integer
weights**. A single K, **0.8806824810924139**, was calibrated from the baseline's
integer scores on training samples only and frozen for all development MSEs.
It is not recalibrated on development labels or separately for each candidate.
The small difference from the fitter's K reflects scoring the deployed integer
blend instead of the fitter's continuous blend.

The helper's layout/conversion was checked against all 611 compiled baseline
weight pairs. Every source installation was also read back and compared with
its intended emitted vector before matching. The actual engine and baseline
fitter were independently rebuilt with identical binary hashes during the audit.

## New data and partitioning

### Prospective parent ownership

All descendants of a parent inherit one channel. The allocation was fixed before
generating games or observing candidate results:

| Channel | Parent seeds |
| --- | --- |
| Training, 32 | Italian, Ruy Lopez, Scotch, Four Knights, Vienna, King's Gambit; all six Sicilian seeds; QGD, QGA, Slav, Semi-Slav, Catalan, Tarrasch; Nimzo, Queen's Indian, King's Indian, Grünfeld, Benoni, Dutch; Scandinavian, Alekhine, Philidor, Petroff; London, Colle, Trompowsky, Stonewall |
| Development, 8 | Three French seeds, two Caro-Kann seeds, Pirc, Modern, Owen |
| Confirmation, 8 | Three English seeds, two Réti seeds, Larsen, Bird, Polish |

The books contain **2,048 training, 512 development and 1,536 confirmation
positions**, in equal per-parent quotas and deterministic round-robin order.
The source is the repository's 48 curated parents, not an unrelated external
opening database. Those families have appeared in the engine's history; this
partition concerns the **new update**, not historical ignorance by the base.

### Generation and canonical identity

The archived Rust generator uses splitmix-derived per-parent/attempt seeds and
xorshift `(13,7,17)`, with sorted UCI moves and modulo selection. It plays a
complete requested prefix of 8–16 quiet legal plies; it excludes promotions,
occupied destinations (including king-to-rook castling representation), en
passant captures, checks, terminal successors and repeated positions within the
prefix. It also avoids entering another channel's original parent position.

Every prefix was replayed. Endpoints are deduplicated across all three books and
against the audited historical inventory. Identity is piece placement, side to
move, castling rights and **effective en passant**; counters are ignored, and an
EP target is cleared when no legal EP capture exists, including pinned-pawn
cases. The generator does not select positions by candidate evaluation.

The inventory covers 30 committed EPD/PGN/gzip files, 22,585 position records and
2,121 distinct canonical starts. These are historical **starting positions and
fixtures**, not every position ever used to train the old engine. The books are
mutually disjoint and contain none of those audited starts. Quiet random
prefixes are legal, not guaranteed strategically balanced.

### Corpus generation and extraction

The frozen base played itself at **Aggression 0**, 200 ms/move, one thread and
16 MiB hash. Training used 4,096 games at concurrency 40; separate development
loss data used 512 games at concurrency 8. Both runs were fault-free. Their
same-binary match Elo is not a strength result.

Training averaged 8.35 completed plies of search per move, development 8.30.
Only training games feed the optimizer. All candidate strength screens use
Aggression 75, as requested.

Extraction skips 16 played plies **after the generated opening** and uses the
existing extractor's non-check/non-promotion/non-ordinary-capture filter. It is
not a proof of tactical quietness. Because that filter can admit en passant,
subsequent filtering conservatively removes positions with any legal EP capture.
Canonical duplicates retain their first occurrence in deterministic source order.

| Corpus | Raw extracted positions | Canonical duplicates removed | Legal-EP positions removed | Forbidden overlaps removed | Retained |
| --- | ---: | ---: | ---: | ---: | ---: |
| Training | 148,368 | 7,623 | 204 | 0 | **140,541** |
| Development loss | 18,335 | 926 | 20 | 12 | **17,377** |

Training excludes historical starts and both validation books. Development loss
samples exclude historical starts, confirmation starts and retained training
samples. There is no identical retained training/development position. The
confirmation book remained unplayed and unscored by candidate engines; only
its generation, replay and identity checks were performed.

The prior model was fitted on approximately **1.84 million older positions**.
This new corpus is deeper-search data but substantially smaller, which is why
strong priors and rare-feature holds were used. It is a pilot, not a justification
for an unconstrained rewrite of all evaluation weights.

## Why no model was promoted

All four models retained the forced tactical, defensive and mating choices in
the standard checks, and complete-game forcing rates stayed close to the base.
The observed differences are not evidence that every changed quiet move is bad
chess. However, existing style checks did not pass unchanged:

- **Conservative:** five endpoint move mismatches. At 75 the short pawn-storm
  position chose `d2b3`; at 100 the equal-queen position exchanged queens.
  The default knight investment remained, but the small match estimate did not
  establish a gain to justify revising the other expectations.
- **Weaker prior:** ten endpoint mismatches, a sacrifice-gate mismatch and the
  loss of the default `e5c6` choice. A tuning-feature test run also failed the
  existing cold/warm TT best-move equality check (`f1c4` versus `b1c3`); 187 tests
  passed, one failed and 108 were not run after fail-fast. This was not relaxed
  or repaired as part of fitting.
- **Hybrid:** four endpoint mismatches and a stronger but still inconclusive
  development match. At 75 the knight fixture chose `d2d4` at 20k, 100k and 400k
  nodes. The pawn storm returned to `b2b4` at 100k, while the 100-profile queen
  exchange persisted through two million nodes. Its 243 tuning-feature library
  tests passed, but that is not a full integration/style verdict.
- **Features-only:** six endpoint mismatches, one sacrifice-gate mismatch and
  the lost default investment. It chose `c1f4` in the unsupported-Greek-gift
  control at 3 cp reference loss against the existing 1 cp cap. Its 243 library
  tests passed; the acceptance suite did not.

No root-risk margin, sacrifice requirement, expected move, node budget or test
was loosened to make a fit qualify. All experimental weight rewrites were undone;
both evaluation files match their pre-experiment hashes exactly. The report does
not claim the fitted models are proven weaker. It says they have not yet earned
deployment under the current strength/style checks.

## Reproduction and audit

The [artifact index](data/evaluation-refit-pilot/sha256.json) covers **127 files**:
all six complete PGNs (8,704 games), manifests and analyses; compressed final
sample corpora; books and lineage; fit declarations/logs/vectors; loss reports;
failed and passing checks; and the helper sources. Executable binaries and
regenerable raw extraction/key files are not bundled.

Run from a checkout with the baseline engine sources and weights at
`8b00cd00323b1c9088675225b7134386edf3be3a`; this documentation-only commit also
retains those weights. Rust/Cargo 1.88.0 and Python 3.12.3 were used. The dependency
is locked to `7e93cdea094a50c1574081ceb6e7b269ad0234ee`.

```sh
export JAKKOO_TEMP="$(mktemp -d)"
ART=docs/tuning/data/evaluation-refit-pilot
python3 "$ART/helpers/build-refit-helpers.py" --out "$JAKKOO_TEMP"
python3 "$ART/helpers/audit-refit-pilot.py" \
  --bin-dir "$JAKKOO_TEMP" --scratch "$JAKKOO_TEMP/audit"
```

The audit was run successfully before export. It verifies artifact hashes,
regenerates and replays the exact books, checks parent ownership and canonical
separation, reparses all games and paired statistics, reconstructs extraction
and filtering byte-for-byte, checks emitted weight vectors, and recomputes all
training/development integer-model losses. It also verifies that the original
engine weights remain in place.

For example, reproduce the conservative fit from the archived training samples:

```sh
gzip -dc "$ART/training.filtered.txt.gz" > "$JAKKOO_TEMP/training.txt"
"$JAKKOO_TEMP/refit-tune-base" fit \
  --positions "$JAKKOO_TEMP/training.txt" --out "$JAKKOO_TEMP/conservative.fit.rs" \
  --holdout 0 --lambda 1 --epochs 200 --rate 0.25 --l2 1e-5 --min-observations 2000
```

The other exact flags are in the archived protocols and fit logs. Always use the
frozen **baseline fitter**: rebuilding it after installing a candidate changes
both initialization and regularization target. Do not compare the raw blended
training loss with the outcome-only loss or use `--epochs 0` as a substitute for
the independent emitted-model scorer.

## Limitations and next research

- Screens share eight development parents; their intervals are not family-level
  or model-selection-adjusted guarantees. Confirmation was deliberately not run.
- Parent-seed separation does not eliminate related structures or all
  transpositions. Historical novelty is limited to the audited inventory.
- The extractor retains the first full-FEN occurrence before canonical filtering;
  it does not average conflicting outcomes for identical positions.
- Fixed-time self-play on a shared 96-logical-CPU Linux host is not bitwise
  reproducible. The archived games and sample corpora provide exact fit inputs.
- Support thresholds count positions, not independent games or phase-specific
  support. Centering/anchoring can move held coordinates in general.
- No external teacher, opponent or tournament-time-control validation was used.

The corpus and unused confirmation book can support a larger or better-labelled
pass, or explicit work on quantization and evaluation/style interaction. The
hybrid model is available for further controlled study. None of those directions
is an additional Elo gain established by this pilot.
