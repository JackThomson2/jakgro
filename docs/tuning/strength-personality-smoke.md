# Strength and personality smoke gate

This report records the cross-channel validation of the search-strength series against repository `HEAD` `a668144eed4b2dc4772f20686a0b5f5e3c8075db`.
It is a **passing fixed-node smoke gate**, not a publishable Elo claim. The comparison includes a baseline Aggression-100-to-0 match so that personality cost is judged relative to the engine being replaced rather than against an arbitrary absolute floor.

Machine-readable output is in [`data/strength-personality-smoke-gate.json`](data/strength-personality-smoke-gate.json).

## Inputs

- candidate SHA-256: `bc746252142ac9e5e5560c6d2f97be91e0c351ab93970e378ecaf841ebccff25`
- baseline SHA-256: `a43eeef16adf6923eed854dd8b94b73d67cad122c555dde208e74006e99b701e`
- match size: 48 games per channel
- search limit: 10,000 nodes per move
- openings: repository `tools/data/openings.epd`, color-reversed pairs
- evidence class: fixed-node smoke only

The gate binds every match summary to its execution manifest and checks that objective and same-profile matches use the same old/new binaries. Candidate and baseline personality matches must each compare two aggression profiles of exactly one binary.

## Results

| Channel | Elo estimate | 95% interval | Gate |
| --- | ---: | ---: | --- |
| Aggression 0 candidate vs baseline | -14.5 | [-238.7, 196.8] | pass at smoke floor |
| Aggression 100 candidate vs baseline | +51.0 | [-150.8, 301.5] | pass at smoke floor |
| Candidate Aggression 100 vs candidate Aggression 0 | -373.8 | [unbounded, -84.0] | relative comparison pass |
| Baseline Aggression 100 vs baseline Aggression 0 | -373.8 | [unbounded, -84.0] | reference |

The candidate's measured personality-cost delta was therefore 0.0 Elo in this sample: the aggressive profile retained its relative cost instead of regressing while the same-profile old/new point estimate moved in the candidate's favor.

The deterministic channels also passed:

- all frozen Aggression 0/100 personality choices were retained;
- forcing-move rate retention was 105.2%;
- all 16 objective/personality positions passed, with a maximum measured root loss of 44 cp;
- all four sacrifice/safety positions passed, with a maximum measured root loss of 37 cp.

The fixed-depth comparison searched 6.06% more nodes geometrically over 15 active positions at depth 4. This is inside the smoke contract's explicit 10% overhead ceiling, but it is a remaining optimization target rather than evidence of a speedup.

## Verdict

The candidate passes the repository's smoke gate: it retains the frozen personality and sacrifice behavior, does not worsen the measured profile cost relative to the baseline, and has a positive same-profile old/new point estimate. The intervals are far too wide for a modest Elo claim. A publishable result still requires a larger opening corpus or an external SPRT, and the continuation/history implementation should be retuned if fixed-depth overhead grows beyond the current bound.
