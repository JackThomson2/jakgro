# Accepted default aggression profile

This note records why Jakgro's default `Aggression` setting is 75 while the full 100 profile remains available as the wilder endpoint. The selection uses the engine series based on `fab28bea5084f5bfacd5c141f1d100a4b0be3f01`; the search behavior measured here ends at `d02f629e5f9dbccd100e86c6e3a43a0e509db225` and is unchanged by changing the advertised default.

## Decision

The default moves from 100 to 75.

At 20,000 nodes per move over 48 color-reversed opening pairs:

| Candidate | Score | Elo point estimate | 95% interval | Forcing moves / 100 |
|---|---:|---:|---:|---:|
| Aggression 75 vs 0 | 41.67% | -58.5 | [-219.2, 79.7] | 29.75 vs 26.90 |
| Aggression 100 vs 0 | 17.19% | -273.2 | [unbounded, -94.0] | 27.55 vs 31.62 |

Profile 75 is the strongest tested profile that clears the new -125 Elo personality-cost point-estimate floor. It also produces about 10.6% more forcing moves per 100 moves than profile 0 on this corpus. Profile 100 remains useful for users who explicitly prefer maximum volatility, but its measured strength cost is too large for the default.

The deterministic profile still preserves the reviewed forcing attack, opposite-side attack, pawn-storm, compensated sacrifice, safety, anti-sacrifice, and simplification controls. Opening development and the Black early-queen sortie now follow the objective move; this is the deliberate style concession that accompanies the stronger default.

## Search-series evidence

The preceding search patches were screened independently against their immediate predecessor:

- personality-neutral search scores: +21.7 Elo over 96 games;
- bounded styled-root verification: +65.9 Elo over 48 games;
- tighter ordinary root risk: +43.7 Elo over 48 games.

Every interval crosses zero. These are regression screens, not additive Elo claims. The accepted profile decision instead relies on the direct 75-versus-0 and 100-versus-0 comparisons above.

The machine-readable evidence is in [`data/default-aggression-75.summary.json`](data/default-aggression-75.summary.json). It records binary and artifact hashes, forcing rates, profile results, and limitations.

## Gate contract

`tests/data/strength-personality-contract.json` now evaluates old-versus-new matches at profile 75 and requires:

- the existing objective and accepted-profile old-versus-new floors;
- personality cost no more than 35 Elo worse than the baseline binary;
- candidate profile-75 personality Elo of at least -125;
- at least 90% forcing-rate retention;
- no deterministic root loss above 45 centipawns;
- no material search-efficiency regression.

The absolute personality floor is intentionally a point-estimate smoke gate. The 96-game interval is too wide to use its lower endpoint as an acceptance threshold.

## Reproduction

Build the measured engine and run the paired profile match:

```sh
cargo build --release --locked
python3 tools/measure_style.py --engine target/release/jakgro --check
python3 tools/measure_acceptance.py --engine target/release/jakgro --check
python3 tools/run_match.py \
  --engine target/release/jakgro \
  --candidate-aggression 75 \
  --candidate-name Current-A75 \
  --baseline-aggression 0 \
  --baseline-name Current-A0 \
  --games 96 \
  --nodes 20000 \
  --pgn artifacts/default-a75-vs-a0.pgn \
  --manifest artifacts/default-a75-vs-a0.manifest.json
python3 tools/analyze_match.py \
  --pgn artifacts/default-a75-vs-a0.pgn \
  --manifest artifacts/default-a75-vs-a0.manifest.json \
  --json artifacts/default-a75-vs-a0.summary.json
```

## Limitations

- Fixed-node games do not model clock management or time losses.
- The deterministic opening set is useful for regression screening but is not a random sample of chess positions.
- The approximate Hoeffding intervals are wide.
- A larger held-out SPRT or time-control match is still required before publishing an Elo improvement claim.
