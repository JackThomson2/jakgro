# Verified root-style margin recalibration

This report validates the engine series ending at
`11432896b34b4666621674605f6b99b05a917543` against the frozen
strength/personality contract `tests/data/strength-personality-contract.json`.
The change under test recalibrates the ordinary root-style margin cap from 30
to 26 centipawns. It was driven by a measurement probe over the frozen
personality suite (`tests/data/personality.epd`) showing the largest
styled-versus-objective cost across all 16 fixtures is 26cp
(`open-king-gambit`), with every other fixture at 14cp or less, so a 26cp cap
preserves every frozen style choice while tightening the bound on positions
that do not need the full swing. Compensated sacrifices keep the uncapped
margin and winning positions keep the tighter 20cp bound.

## Binaries under test

- candidate: margin 26, `ORDINARY_ROOT_MARGIN_MAX = 26`
  (`HEAD`, commit `11432896`), sha256
  `2ae40a45209dfd37c8972299d7a9d7863910a4fa6412bed210d08f2a2c5bbb93`
- baseline: margin 30, `ORDINARY_ROOT_MARGIN_MAX = 30`
  (`HEAD~1`, commit `677eaa98`), sha256
  `f1cf676bd1284fc348b3f83cd730b79d29d54e517f6a784ae9383a9e4159f72a`

Both binaries were built from clean trees (candidate from the committed
worktree, baseline via `git archive HEAD~1`), so the only behavioral
difference is the margin cap.

## Gate result: PASS

`tools/gate_strength_personality.py` against the frozen contract reported
overall `passed: true`. Per-channel:

- objective (Aggression 0 vs 0, 48 games): Elo 0.0, 48 draws. The objective
  search is bit-identical, as required for the `Aggression=0` control.
- same-profile (new 75 vs old 75, 48 games): Elo +29.0, CI95
  [-177.7, +262.1]. A point estimate in the intended direction; the interval
  is wide at 48 games, so this is smoke evidence, not a measured gain.
- personality comparison: candidate (new 75 vs 0) -120.4 Elo vs baseline
  (old 75 vs 0) -104.4 Elo, delta -16.0 (>= -35) and candidate -120.4 (>=
  -125). The personality cost stays within the frozen bound.
- style: 16/16 expected moves and 4/4 sacrifice controls preserved at both
  profiles; forcing-move retention 106.9% (>= 90%). The aggressive identity
  is intact.
- acceptance: objective-personality 16/16 (max root loss 44cp <= 45) and
  sacrifice-acceptance 4/4 (max root loss 37cp <= 45).
- efficiency: geometric node reduction -0.18% (>= -10%), behavior-neutral as
  expected for a margin-only change.

## Interpretation

The recalibration is behavior-safe: it keeps every frozen style and control
choice, holds the personality cost within the contract bound, and leaves the
objective control bit-identical. The +29 Elo same-profile point estimate is
directionally consistent with recovering strength by spending less style
margin on quiet positions, but at 48 games the confidence interval includes
zero, so it is not a statistically established gain. A longer match would be
needed to measure the true effect size.

## Artifacts

- gate output: `docs/tuning/data/recalibrated-margin-gate.json`
- same-profile match summary:
  `docs/tuning/data/recalibrated-margin-same-profile.summary.json`
- personality match summary:
  `docs/tuning/data/recalibrated-margin-personality.summary.json`
