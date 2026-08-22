# Aggression 100 versus 0 baseline

This report asks whether the tuned attacking profile at Aggression 100 preserves playing strength against the same engine with all style terms disabled at Aggression 0. It is an initial deterministic fixed-node baseline, not an Elo proof or an SPRT result.

## Immutable inputs

| Input | Value |
| --- | --- |
| Repository revision used to build the engine | `0839dbfdac905e42f01ae1cf5ee743042907ff01` |
| Candidate and baseline release binary SHA-256 | `e6b4448eb8bdd4f1d1ef86730e94e2ddce712f01d93c6f218319ff9c0ee57de6` |
| Candidate setting | Aggression 100 |
| Baseline setting | Aggression 0 |
| Opening corpus | 48 unique, color-reversed EPD positions |
| Opening corpus SHA-256 | `8f67f7bdb3c659140516e9f692694ca8513633ee0d9302374dccef927eaa0cde` |
| Match runner SHA-256 | `cfb72dcf9707dcc7b2c68a41e2b5c11dc5ae277b6a423e5a67fa1dd9cb8c0c31` |
| Cutechess release | `cutechess-cli 1.5.1` |
| Cutechess source | tag `v1.5.1`, commit `45e923949e43570886c0ad3392f514e743839c6b` |
| Cutechess binary SHA-256 | `bb8ec8df71ce0ef95ec03614440fe93c31730bbc0c8fbfd07a535e14b7b5d550` |
| Runtime | Qt 6.11.1 on macOS 26.5 arm64, Apple M2 Pro |

The official cutechess source was built in release mode against Homebrew Qt because Homebrew did not provide a `cutechess` formula. The resulting CLI was installed as `/opt/homebrew/bin/cutechess-cli` after its version and hash were captured.

## Match settings

- Paired games with colors reversed for every opening.
- One concurrent game.
- 50,000 nodes per move for the full baseline.
- 16 MiB Hash per engine.
- Sequential EPD opening order.
- Draw adjudication after move 80 when both scores remain within 10 centipawns for 10 moves.
- Two-sided resignation after four moves at an 800-centipawn score.
- Maximum 200 moves per game.
- The candidate and baseline are separate processes running the same binary; only `Aggression` differs.

## Protocol pilot

A four-game pilot at 5,000 nodes per move completed without a crash, disconnect, incomplete PGN, or manifest mismatch.

- W/D/L for Aggression 100: **3/0/1**.
- Score: **75.00%**.
- PGN SHA-256: `32c28faadb230c2b83327d38196ef32f0741dc5f39ae8c6a877778dcda84870e`.
- Manifest SHA-256: `3ac92f93b93559605286a1d1db37f41f78e36de8843c25f2e89d7141b340a168`.

The pilot is only a protocol gate; its sample is too small for tuning conclusions.

## Endpoint style gate

The release binary passed all 12 fixed-node endpoint observations before the match:

- Aggression 0 retained the three base-profile choices.
- Aggression 100 retained the four reviewed attacking choices, including the color-swapped king-pressure case.
- Both profiles retained the tactical-safety and defensive-safety choices.
- No expected move, score determinism, or legal-result gate failed.

## Full 96-game baseline

The full 50,000-node match completed all 96 games in 380.47 seconds. The manifest reported return code 0, the analyzer accepted every reversed-color pair, and the PGN hash matched the manifest.

- W/D/L for Aggression 100: **35/15/46**.
- Score: **44.27%**.
- Decisive games: **84.38%**.
- Approximate Elo difference: **-40.0**.
- Approximate 95% score interval: **35.25% to 53.29%**.
- Approximate 95% Elo interval: **-105.6 to +22.9**.
- Average game length: **102.23 plies**.

### Color split

| Candidate color | W | D | L | Score |
| --- | ---: | ---: | ---: | ---: |
| White | 19 | 7 | 22 | 46.88% |
| Black | 16 | 8 | 24 | 41.67% |

Across the 48 opening pairs, Aggression 100 scored 0, 0.5, 1, 1.5, and 2 points in 10, 9, 17, 6, and 6 pairs respectively. Seventeen pairs split decisively and none produced two draws. Cutechess recorded 74 adjudications and omitted a termination label for 22 games.

- PGN SHA-256: `db04f19fcc8713ad0bc1f541a05d5ec6ad72e7daaa3645a53b6621f35ae213db`.
- Manifest SHA-256: `479bf5b0eb83e7be603cb265069252fa96fb8efbacc9c5f56bebc792adbe25d8`.
- Deterministic analyzer output: [`data/aggression-100-vs-0.summary.json`](data/aggression-100-vs-0.summary.json).

## Interpretation and next experiment

The point estimate indicates a meaningful strength cost at Aggression 100, especially when playing Black. The interval still crosses 50% and zero Elo, so this baseline does not establish a statistically conclusive loss, but it also does not support a strength-improvement claim. The high decisive-game rate and unchanged style gate show that the profile is materially changing play rather than merely shifting reported scores.

No evaluation weights are changed in this measurement series. The next tuning experiment should compare Aggression 75 with Aggression 0 on the same corpus, then compare 100 with 75 only if 75 preserves the reviewed attacking choices. Any weight change should use separate development openings and retain held-out pairs for confirmation.

## Reproduction

Build the exact engine revision and run:

```sh
cargo build --release --locked
python3 tools/measure_style.py --engine target/release/jakgro --check
python3 tools/run_match.py \
  --engine target/release/jakgro \
  --cutechess /opt/homebrew/bin/cutechess-cli \
  --games 96 \
  --nodes 50000 \
  --pgn artifacts/aggression-100-vs-0.pgn
python3 tools/analyze_match.py \
  --pgn artifacts/aggression-100-vs-0.pgn \
  --json artifacts/aggression-100-vs-0.summary.json \
  --markdown artifacts/aggression-100-vs-0.summary.md
```

## Limitations

- Fixed-node games do not model real clock management or time forfeits.
- The 48 opening pairs are deterministic and deliberately varied, but they are not a random sample of all chess positions.
- The reported confidence interval is an approximate normal interval over paired opening scores.
- A single baseline should not drive weight changes. Any follow-up tuning should change one factor at a time and reserve held-out openings for confirmation.
