# Measured search-efficiency and fixed-time smoke result

This report evaluates the engine series ending at
`d2be2cfe5bafc835a2752b58b153959818291509` against baseline
`3b93635e0d76aab36f4b3b91ee7380c3f1928ffc`. Both release binaries used the
same `cozy-chess` checkout at
`7e93cdea094a50c1574081ceb6e7b269ad0234ee`; that checkout had no tracked or
staged changes. The candidate and baseline SHA-256 values were respectively
`3a27c96788e1a05921b49386fe28fba4a299e633bc591ad7410926e21ac0e159` and
`f0afb212eb1f64b36cf6766c2864ebe80789687e27100af55fd61f57958d85c2`.

## Search efficiency

The paired ten-position suite used seven samples per timed observation and a
500 ms fixed-time window. All positions were active, repeatable, and reported
throughput in both profiles.

| Aggression | Fixed-depth node reduction | Fixed-node NPS change | Fixed-time depth gain | Gate |
|---:|---:|---:|---:|---|
| 0 | 8.209% | -0.487% | +0.100 ply | Pass |
| 75 | 5.406% | -0.413% | 0.000 ply | Pass |

The result is a search-efficiency improvement rather than a raw hot-path speed
improvement: capture history searches a smaller tree while adding less than
one percent of measured per-node cost. The deterministic benchmark also showed
that deferred terminal detection reduced full legal-move probes from 116,553
to 58,751 across its seven fixtures, while preserving the same aggregate node
count for that behavior-preserving change.

The machine-readable evidence is in:

- `data/search-elo-a0-efficiency.json`;
- `data/search-elo-a75-efficiency.json`;
- `data/search-elo-benchmark.csv`.

## Fixed-time strength smoke

Cute Chess 1.5.1 ran 96 games per profile from 48 paired openings at
`0.25+0.002`, 16 MiB hash, and concurrency one. Candidate and baseline used the
same aggression setting in each match.

| Aggression | Candidate W-D-L | Score | Elo point estimate | Paired 95% interval |
|---:|---:|---:|---:|---:|
| 0 | 48-29-19 | 59.90% | +69.68 | [-68.32, +235.43] |
| 75 | 45-36-15 | 54.69% | +32.67 | [-106.89, +184.33] |

Both point estimates are positive, but both intervals include zero. These runs
are therefore positive smoke evidence, not statistically conclusive Elo gains.
A verified Elo claim requires extending the paired matches until the interval
excludes zero or applying a predeclared sequential test. The retained summaries
are `data/search-elo-a0-match.json` and `data/search-elo-a75-match.json`.

Capture-history influence is fully enabled through the default Aggression 75
profile and tapers to zero at Aggression 100. The maximum-aggression regression
suite, deterministic search fixtures, null-move contract, and acceptance
contracts all passed, so the measured node gains do not replace or weaken the
engine's existing personality gates.

## Commands

The efficiency measurements used the documented paired workflow with explicit
`--aggression 0` and `--aggression 75`, `--samples 7`, `--move-time-ms 500`, and
`--check`. The fixed-time matches used:

```sh
python3 tools/run_match.py \
  --engine /path/to/candidate/jakgro \
  --baseline-engine /path/to/baseline/jakgro \
  --candidate-aggression PROFILE \
  --baseline-aggression PROFILE \
  --games 96 \
  --time-control 0.25+0.002 \
  --hash 16 \
  --candidate-revision d2be2cfe5bafc835a2752b58b153959818291509 \
  --baseline-revision 3b93635e0d76aab36f4b3b91ee7380c3f1928ffc \
  --dependency-revision 7e93cdea094a50c1574081ceb6e7b269ad0234ee \
  --build-profile release \
  --pgn match.pgn \
  --manifest match.manifest.json

python3 tools/analyze_match.py \
  --pgn match.pgn \
  --manifest match.manifest.json \
  --json match.summary.json
```
