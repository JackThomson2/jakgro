# Wilder Aggression profile

This note defines the validation contract for the more adventurous high end of Jakgro's `Aggression` option. It supplements the historical [Aggression 100 versus 0 baseline](aggression-100-vs-0.md); it does not rewrite or reinterpret that earlier result.

## Intended behavior

Aggression 0 remains the conventional control. Aggression 100 now combines six deliberate biases:

1. coordinated king attacks, supported threats, open lines, and pawn breaks without a generic static refund for material deficits;
2. larger check-extension and quiet-check quiescence budgets;
3. earlier ordering of checks and advanced pawns near the enemy king;
4. legal exchange analysis that distinguishes a real material investment from an immediately recoverable trade;
5. deterministic root selection after the opponent's fully searched reply, with compensation measured on the settled exchange position; and
6. aversion to immediate draws and balanced major-piece simplification when an eligible live alternative exists.

The root selector now applies one hard searched-score guard to every styled candidate. At Aggression 100 that guard is the ordinary 120-centipawn margin; verified sacrifices no longer receive the former 220/380/450-centipawn exceptions. Truncated exchanges, declined offers, unsafe kings, attacks without legal checking resources, and apparent sacrifices erased by recapture receive no sacrifice preference.

The root selector never replaces a mate score with a centipawn score. Its entertainment value is not added to the UCI score, and the conventional result remains the root transposition-table value. If verification is interrupted, search keeps the completed conventional result.

## Deterministic personality gate

`tests/data/personality.epd` contains fixed-node profile choices grouped into initiative, forcing-attack, king-attack, pawn-storm, sacrifice, anti-sacrifice, simplification, and safety categories. The suite includes color-mirrored opening pressure, an opposite-castling attack, a kingside pawn storm, a bishop offer on f7, unsupported Greek Gift and rook offers, avoidance of an equal queen trade, hanging-major-piece controls, forced mate, and forced defense.

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

Acceptable move sets are explicit in the EPD rather than inferred from a fresh engine run. At least half of the personality positions must separate Aggression 100 from Aggression 0. Every `safety` and `anti-sacrifice` position must retain the same move at both endpoints, while the `sacrifice` and `simplification` categories must demonstrate intentional endpoint differences.

The debug-build fixed-node gate for this series completed 32 endpoint searches with zero mismatches. Eight of the 16 positions selected different endpoint moves, while five safety controls and two anti-sacrifice controls remained unchanged; the sacrifice and simplification categories each supplied an intentional profile difference. This is deterministic personality evidence, not game-play or strength evidence.

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
  --markdown artifacts/wilder-vs-previous.summary.md \
  --min-elo-lower-bound 0
```

Checks, captures, promotions, forcing-move rates, verified-sacrifice choices, decisiveness, and game length must be reviewed together with complete games and the same-profile strength result. Higher spectacle counts alone are insufficient, and the paired 95% Elo lower bound must exceed zero before an Elo improvement is claimed.

The completed result is recorded in [Verified aggression: style and strength result](verified-aggression-elo.md). The same-profile candidate scored 84.375% over 96 games and passed the conservative lower-bound gate while increasing all four forcing-play proxies. All sacrifice and control choices were preserved, but the frozen sacrifice hit delta was zero; broader sacrifice improvement therefore remains unconfirmed.
