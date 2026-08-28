# Measured strength series

## Verdict

Two sequential tests confirm this series. Against the pre-series build at a fixed
50 ms per move over 4096 colour-reversed games, the head measures **+108.1 Elo
[99.6, 116.8] at Aggression 75** and **+203.3 Elo [193.6, 213.3] at Aggression
0**. Both cross the predeclared `elo0=0, elo1=20, alpha=beta=0.05` H1 boundary,
with log-likelihood ratios of 118.8 and 261.9 against a bound of 2.94.

A third test confirms the result under a real clock rather than a fixed move
time. At `1.0+0.01` over 1200 games at Aggression 75 the head measures **+124.7
Elo [108.0, 141.9]**, also accepting H1, with no time forfeits and no protocol
faults recorded. The gain is therefore not an artefact of the short fixed move
time used for throughput, and it survives clock management.

This is an Elo claim rather than smoke evidence: every paired 95% interval
excludes zero by a wide margin, which is the standard
[`search-performance.md`](search-performance.md) sets.

The default profile keeps its personality. Over 600 fixed-node games against
objective play, forcing-move retention is 100.2% against a 90% floor, and the
head plays 11.19 checks per hundred moves against 9.05 for objective play. The
measured cost of the default profile narrows from -152 Elo before the series to
-115 after it, so the attacking profile became cheaper as well as stronger.

## Provenance

| Input | Value |
| --- | --- |
| Baseline revision | `4a321dbc771fcd44d1478e4aff8ea7547b67d7fb` |
| Baseline binary SHA-256 | `e13ca3b0d83afc75…` |
| Candidate binary SHA-256 | `94ecfe7634d74333…` |
| Dependency revision | `7e93cdea094a50c1574081ceb6e7b269ad0234ee` |
| Build profile | `release` |
| Opening corpus | `data/selective-search-confirmation.epd`, 2048 positions, `9f44058d…` |
| Match limit | 50 ms per move, 16 MiB hash, concurrency 44; clocked confirmation at `1.0+0.01` |
| Artifacts | `data/strength-series-a75.sprt.json`, `data/strength-series-a0.sprt.json`, `data/strength-series-a75-clocked.sprt.json`, `data/strength-series-lmp-isolated.sprt.json` |

Matches ran through `tools/run_sprt.py` and the `selfplay` arbiter added by this
series, because this host has no `cutechess-cli`. Every summary is bound to its
manifest, binaries, and corpus by SHA-256 in the same scheme `run_match.py` uses.

## Per-patch results

Each patch was measured against its immediate parent, so these are independent
verdicts rather than a decomposition of the total. Fixed-time matches use 60 ms
per move unless stated.

| Patch | Aggression 75 | Aggression 0 | Mechanism |
| --- | --- | --- | --- |
| Repetition scan | +5.7 [0.1, 11.2] | not measured | +1.46% NPS, tree identical |
| Swap-list SEE | +13.8 [5.4, 22.1] | +117.2 [107.8, 126.7] | 37.0% fewer nodes at A0 |
| Fused extraction | +20.6 [12.3, 29.0] | +23.1 [15.1, 31.1] | +12.5% and +14.4% NPS, tree identical |
| Piece-square tables | +55.2 [43.8, 66.7] | +56.6 [45.0, 68.4] | new evaluation knowledge |
| Styled root budget | +5.9 [-2.9, 14.8] | not measured | enabler, neutral by design |
| Late-move table | +0.5 [-7.0, 8.0] | +10.7 [3.3, 18.1] | 21.1% fewer nodes, +0.40 ply |

The last row combines two changes, and the pruning half was measured separately
because its default-profile verdict spans zero. Move-count pruning alone, toggled
against an otherwise identical head over 4096 games at 60 ms per move, measures
**+2.7 Elo [-4.3, +9.7]** at Aggression 75. That interval bounds the harm rather
than proving a gain: the pruning is worth +10.7 Elo at the objective profile and
is at worst neutral at the default one, which is why it ships unconditionally
rather than being gated on aggression. The reduction half carries the row's
node and depth effect at both profiles.

Two patches were implemented, measured, and **rejected** rather than shipped:

- **Transposition prefetch and wider static-evaluation storage.** Both halves
  were tree-identical but slower, 0.981x and 0.985x at Aggression 75. The child
  key is computed immediately before the recursive call, so a prefetch has no
  work to overlap with.
- **A personality-neutral king-danger term.** A non-linear attacker-count curve
  measured -14.0 Elo [-23.7, -4.4] at Aggression 75 at weight `(3, 1)` and -5.5
  at weight `(1, 0)`. The existing shelter and open-file terms plus the new king
  piece-square table appear to cover the same signal already.

## Cumulative search behaviour

Measured over the frozen `tests/data/search-performance.epd` suite, head against
the pre-series build:

| Channel | Aggression 75 | Aggression 0 |
| --- | --- | --- |
| Fixed-depth node reduction, depth 8 | 44.73% | 68.25% |
| Fixed-node throughput | 1.086x | 0.999x |
| Completed depth at 500 ms | +1.100 ply | +1.300 ply |

Throughput at Aggression 0 is flat because the evaluation speedups are offset by
static exchange evaluation now running at every profile, which is what buys the
node reduction in the same column.

## Limitations

- All 2048 confirmation positions descend from the 48 curated opening families,
  so they are not an independent opening book.
- Elo is measured against this repository's own previous build. It says how much
  the engine improved, not where it stands against other engines.
- The main channels use a 50 ms move time for throughput. The `1.0+0.01`
  confirmation shows the gain holds under a real clock at roughly twenty times
  that budget, but nothing here reaches tournament time controls.
- No work was done on clock management itself, so the soft and hard budget ratios
  are unchanged from before the series.
- Fixture expectations were rebaselined repeatedly during the series. Every
  change is recorded in its commit, but the suites no longer pin the same moves
  they did before, and the sacrifice control was replaced twice.
- The sequential test uses a normal-approximation log-likelihood ratio. It is
  unit-tested against closed-form expectations and its boundaries, but it was not
  cross-validated against an independent SPRT implementation.

## Reproduction

```sh
cargo build --release --locked --bin jakgro --bin selfplay
python3 tools/run_sprt.py \
  --engine target/release/jakgro \
  --baseline-engine /path/to/baseline/jakgro \
  --candidate-aggression 75 --baseline-aggression 75 \
  --games 4096 --movetime-ms 50 --concurrency 44 \
  --elo0 0 --elo1 20 \
  --openings docs/tuning/data/selective-search-confirmation.epd \
  --pgn artifacts/strength-series-a75.pgn \
  --summary-json artifacts/strength-series-a75.sprt.json
```

Swap both `--candidate-aggression` and `--baseline-aggression` to `0` for the
objective channel, or replace `--movetime-ms 50` with `--time-control 1.0+0.01`
for the clocked confirmation. The deterministic gates run as before:

```sh
python3 tools/measure_style.py --engine target/release/jakgro --check
python3 tools/measure_style.py --engine target/release/jakgro \
  --suite tests/data/sacrifice-gates.epd --check
python3 tools/measure_acceptance.py --engine target/release/jakgro --check
python3 tools/validate_acceptance_contract.py
cargo nextest run --locked
cargo bench --locked --bench search
```
