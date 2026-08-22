# Wilder Aggression profile

This note defines the validation contract for the more adventurous high end of Jakgro's `Aggression` option. It supplements the historical [Aggression 100 versus 0 baseline](aggression-100-vs-0.md); it does not rewrite or reinterpret that earlier result.

## Intended behavior

Aggression 0 remains the conventional control. Aggression 100 now combines four deliberate biases:

1. coordinated king attacks, supported threats, open lines, pawn breaks, and bounded compensation for material deficits in static evaluation;
2. larger check-extension and quiet-check quiescence budgets;
3. earlier ordering of checks and advanced pawns near the enemy king; and
4. deterministic root selection among fully verified alternatives no more than 120 centipawns below the conventional result.

The root selector never replaces a mate score with a centipawn score. Its entertainment value is not added to the UCI score, and the conventional result remains the root transposition-table value. If verification is interrupted, search keeps the completed conventional result.

## Deterministic personality gate

`tests/data/personality.epd` contains fixed-node profile choices grouped into initiative, forcing-attack, king-attack, pawn-storm, and safety categories. The suite includes color-mirrored opening pressure, an opposite-castling attack, a coordinated queen-and-bishop attack, a kingside pawn storm, hanging-major-piece controls, forced mate, and forced defense.

Run both the UCI and engine-API gates with:

```sh
cargo build --release --locked
python3 tools/measure_style.py \
  --engine target/release/jakgro \
  --profiles 0,100 \
  --check \
  --summary-json artifacts/wilder-style.summary.json
cargo test --test aggression_profile
cargo test --test search_regression
```

Acceptable move sets are explicit in the EPD rather than inferred from a fresh engine run. At least half of the personality positions must separate Aggression 100 from Aggression 0, while every position categorized as `safety` must retain the same move at both endpoints.

The debug-build validation run for this series completed 24 searches with zero mismatches. Six of the 12 positions selected different endpoint moves, and all five safety controls retained the same move. This is a deterministic personality gate, not game-play or strength evidence.

## Old-versus-new match protocol

A claim that this profile is *more* aggressive must compare it with a binary built before the profile changes, with both engines set to Aggression 100:

```sh
python3 tools/run_match.py \
  --engine target/release/jakgro \
  --candidate-name Wilder-Aggression-100 \
  --candidate-aggression 100 \
  --baseline-engine /path/to/jakgro-before \
  --baseline-name Previous-Aggression-100 \
  --baseline-aggression 100 \
  --games 96 \
  --nodes 50000 \
  --pgn artifacts/wilder-vs-previous.pgn
python3 tools/analyze_match.py \
  --pgn artifacts/wilder-vs-previous.pgn \
  --json artifacts/wilder-vs-previous.summary.json \
  --markdown artifacts/wilder-vs-previous.summary.md
```

Checks, captures, promotions, forcing-move rates, decisiveness, and game length should be reviewed together with a fixed sample of complete games. No Elo floor is required: reduced playing strength is an accepted tradeoff. Conversely, higher check or capture counts alone are not sufficient evidence that the games are more interesting.

No old-versus-new match result is recorded here. A result should only be added after the exact binaries, opening corpus, PGN, manifest, hashes, and complete analyzer output are available; placeholder or partial data must not be presented as validation.
