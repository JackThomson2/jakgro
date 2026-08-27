# Selective-search tuning and Elo evidence

## Verdict

The tuned series now has a **measured short-time-control Elo gain at the default Aggression 75 profile**.

The final mobility patch beat its immediate generic-mobility parent by **+25.3 +/- 18.5 Elo** over 881 games and crossed the configured SPRT upper boundary, accepting H1 for the `[0, 15]` Elo test. Against the original pre-series baseline, the complete candidate scored **+10.4 +/- 8.4 Elo** over the full 4,096-game confirmation cap with **99.2% likelihood of superiority**. That overall run did not cross the SPRT H1 boundary because its estimate settled below the 15 Elo alternative, but Cute Chess's reported interval remained above zero.

This is evidence at `0.05+0.0005` on the committed confirmation corpus. It is not a claim about longer tournament controls, other hardware, or every Aggression profile. Aggression 0 and 100 intentionally retain the old mobility evaluation; their deterministic personality behavior is unchanged.

## Provenance

| Input | Value |
| --- | --- |
| Final engine candidate | `fe96f0011475c992616dfa507ae7474376c96732` |
| Mobility patch parent | `02cf89969e51248ad11644629e4697b8b67c9a0f` |
| Pre-series baseline | `e17a098089ab0a34daff1c8cd175abe3d8b35e08` |
| `cozy-chess` dependency | `7e93cdea094a50c1574081ceb6e7b269ad0234ee` |
| Candidate binary SHA-256 | `3e475df417fed0b3cf6c0f844eaf38f29b3e50e846e6fe0b6e78537b8281ed22` |
| Mobility-parent binary SHA-256 | `854033dabf4cb8e5b750dc978cfc96ad068cf045e8bb2f722f33827fa1ad791a` |
| Pre-series binary SHA-256 | `866f1012128f1b9ff9ea0d18dcd4cba5dafed79629dd2841de244e993ee0666a` |
| Build profile | `release` |
| Toolchain | `rustc 1.96.0 (ac68faa20 2026-05-25)`, `cargo 1.96.0` |
| Host | Apple M2 Pro, arm64 macOS |

The dependency checkout had untracked local administration files but no tracked or staged modifications. Each named engine was copied to an immutable path before its matches.

## Changes measured

The final engine combines five independently reviewable changes:

1. selective-search telemetry for capture-history, quiescence-pruning, and shallow-LMR attribution;
2. bounded capture-history rewards for previously unproven first-capture cutoffs;
3. previous-move recapture context on the first quiescence ply;
4. conservative child-depth-two LMR, with the weaker child-depth-one horizon reduction removed after strength screening; and
5. per-piece mobility accounting with a profile adjustment that peaks at Aggression 75 and fades to zero at Aggression 0 and 100.

The mobility adjustment converts the old uniform `3/2` middle-game/end-game mobility weight into effective default-profile weights of `4/4` for knights, `5/5` for bishops, `2/4` for rooks, and `1/2` for queens, while removing pawn and king pseudo-mobility from that profile. The ordinary attacking-style channel is unchanged. Objective search retains the selected mobility profile even while style scoring itself is suppressed.

## Tuning protocol and rejected candidates

The search and evaluation choices were screened before the final confirmation run:

- horizon-LMR thresholds of 48 and 64, plus complete removal, were matched against the accepted threshold-32 engine; removal had the best screen and was retained;
- the removal candidate scored +6.1 Elo against threshold 32 on a 512-game generated holdout, but remained neutral against the pre-series engine;
- medium and strong rook-file bonuses were screened; the apparently strong screen reversed to -12.9 Elo on the 512-game holdout and was rejected;
- the selected piece-specific mobility weights scored +27.9 Elo on that holdout before the confirmation corpus was generated;
- weaker and uniform mobility variants did not reproduce the gain and were rejected.

The committed confirmation corpus contains 2,048 unique EPD positions. It was generated after mobility selection from the repository's 48 curated opening seeds using a fixed xorshift seed (`1122334455667788`), two to ten quiet legal continuation plies, rejection of checked positions, and color-reversed pairing. Its SHA-256 is `9f44058d953324751ed1db1a3325f8bfa8ccf31ee6f6cffa3e1e98a391a9de37`.

This procedure separates candidate selection from the final corpus, but all generated positions still descend from the same 48 opening families. The evidence is therefore stronger than reusing the screening games, but not equivalent to an unrelated external opening book.

## Confirmed Elo result

Both confirmation matches used `0.05+0.0005`, 16 MiB hash tables, four concurrent games, sequential EPD order, color reversal, and an SPRT configured with `elo0=0`, `elo1=15`, `alpha=0.05`, and `beta=0.05`.

| Comparison | W/D/L | Score | Elo | Reported interval | LOS | SPRT result |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| Mobility patch vs parent | 318/309/254 | 53.63% | **+25.3** | +6.8 to +43.8 | 99.6% | H1 accepted at 881 games, LLR 3.00 |
| Final series vs pre-series | 1339/1540/1217 | 51.49% | **+10.4** | +2.0 to +18.8 | 99.2% | max 4,096 games, LLR 2.32; no boundary |

The incremental result establishes that the selected mobility change is stronger than its parent under the configured SPRT. The overall result establishes a positive measured effect versus the original baseline under Cute Chess's reported error model, while honestly remaining inconclusive for the stricter 15-Elo H1 target.

## Paired search efficiency

The repository efficiency suite used ten frozen positions at depth 4, seven alternating samples per engine, and a 500 ms fixed-time probe.

| Aggression | Node reduction | NPS change | Mean depth change | Result |
| ---: | ---: | ---: | ---: | --- |
| 0 | +0.774% | -0.921% | +0.200 | pass |
| 75 | +2.222% | -1.621% | +0.200 | pass |

All positions were active and repeatable, retained their expected moves, and reported throughput. The tuned series prioritizes strength over the previous 2%-at-both-profiles target: Aggression 0 no longer clears that old node floor, while the default profile still does.

The standalone seven-fixture benchmark used **2.371% more geometric nodes** than the previously recorded baseline. Per-position movement ranged from a 28.9% reduction in the defensive rook fixture to a 28.5% increase in the central-queen fixture. This is why the strength evidence, paired efficiency suite, and throughput measurements are reported separately rather than treating node count alone as Elo.

## Fixed-node strength and personality gate

Four 96-game, color-reversed matches used 20,000 nodes per move. The repository's combined gate passed:

| Channel | Candidate profile | Baseline profile | Elo estimate | 95% Elo interval |
| --- | ---: | ---: | ---: | ---: |
| Objective old/new | 0 | 0 | 0.0 | -143.9 to +143.9 |
| Default old/new | 75 | 75 | +84.9 | -53.4 to +258.5 |
| Candidate personality | 75 | 0 | +21.7 | -118.9 to +170.4 |
| Baseline personality | 75 | 0 | -69.7 | -235.4 to +68.3 |

The candidate personality comparison improved by +91.4 Elo relative to the baseline estimate. Deterministic endpoint moves and controls were preserved, forcing retention was 116.117%, the maximum objective root loss was 44 cp, and the combined gate passed. These fixed-node values are smoke estimates with wide intervals, not additional Elo claims.

## Fixed-time repository smoke

The conventional repository matches used 96 games, 48 color-reversed opening pairs, `0.25+0.002`, and 16 MiB hash tables.

| Aggression | W/D/L | Score | Elo estimate | 95% Elo interval |
| ---: | ---: | ---: | ---: | ---: |
| 0 | 47/5/44 | 51.56% | +10.9 | -131.3 to +156.9 |
| 75 | 38/22/36 | 51.04% | +7.2 | -135.4 to +152.6 |

These short 96-game checks are directionally positive but statistically inconclusive. The larger confirmation match is the relevant strength result.

## Validation

The final engine content passed:

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo bench --locked --bench search
python3 -m unittest discover -s tools/tests -p 'test_*.py'
```

Deterministic style and acceptance measurements passed against the pre-series binary. The paired efficiency runs used `--samples 7`, `--move-time-ms 500`, and `--check`. Fixed-node and fixed-time PGNs were analyzed against their manifests before the strength/personality gate was evaluated.

The confirmation invocation used the committed EPD and the following essential Cute Chess settings:

```sh
cutechess-cli \
  -each proto=uci tc=0.05+0.0005 option.Hash=16 \
  -rounds 2048 -games 2 -repeat -concurrency 4 \
  -openings file=selective-search-confirmation.epd format=epd order=sequential \
  -sprt elo0=0 elo1=15 alpha=0.05 beta=0.05
```

## Committed artifacts

- [`selective-search-confirmation.epd`](data/selective-search-confirmation.epd)
- [`selective-search-mobility-a75-sprt.json`](data/selective-search-mobility-a75-sprt.json)
- [`selective-search-a75-sprt.json`](data/selective-search-a75-sprt.json)
- [`selective-search-benchmark.csv`](data/selective-search-benchmark.csv)
- [`selective-search-a0-efficiency.json`](data/selective-search-a0-efficiency.json)
- [`selective-search-a75-efficiency.json`](data/selective-search-a75-efficiency.json)
- [`selective-search-a0-fixed-time.json`](data/selective-search-a0-fixed-time.json)
- [`selective-search-a75-fixed-time.json`](data/selective-search-a75-fixed-time.json)
- [`selective-search-strength-personality-gate.json`](data/selective-search-strength-personality-gate.json)

The SPRT summaries bind binary, opening, PGN, and log hashes. The fixed-time summaries bind PGN and manifest hashes. The strength/personality gate binds its four match channels, manifests, style and acceptance summaries, contract, and paired efficiency artifact.
