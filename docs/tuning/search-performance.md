# Paired search-performance measurement

Search changes are evaluated in three separate channels because no single metric
proves that an engine became faster:

- **fixed depth** compares searched nodes and detects changes to tree efficiency;
- **fixed nodes** compares median NPS and detects CPU hot-path changes;
- **fixed time** compares completed depth, the user-visible result.

The frozen `tests/data/search-performance.epd` suite covers openings, quiet and
tactical middlegames, king attacks, forced tactics, and pawn and piece endgames.
Its fixed-node limits are intentionally longer than the correctness fixtures so
that timer quantization does not dominate throughput measurements.

## Reproducible workflow

Build the baseline and candidate from their respective revisions and keep them
at distinct paths. Then run both binaries on the same otherwise-idle machine:

```sh
python3 tools/measure_search_efficiency.py \
  --engine /path/to/candidate/jakgro \
  --baseline-engine /path/to/baseline/jakgro \
  --samples 7 \
  --move-time-ms 500 \
  --summary-json search-performance.json \
  --check
```

The tool warms each engine, alternates which binary runs first, takes the median
fixed-node and timed sample for every position, and reports geometric aggregate
node and NPS ratios. Binary and suite SHA-256 identities are embedded in the
JSON so the evidence remains tied to the measured inputs.

Use the same hash size, aggression, compiler profile, CPU power mode, and system
load for both binaries. Increase `--samples` and `--move-time-ms` when a result
is close to a threshold. A baseline-versus-itself run should center near an NPS
ratio of 1.0 and zero completed-depth gain.

Wall-clock gates are deliberately not part of CI: shared runners cannot provide
repeatable timing. CI continues to enforce deterministic expected moves,
legality, fixed-node regressions, style, and acceptance contracts. Paired local
results supplement those checks; they do not replace strength matches.
