# Jakgro

Jakgro is a Rust chess engine aimed at playing aggressive, tactical, and interesting chess while remaining compatible with the Universal Chess Interface (UCI).

> **Current status:** Jakgro runs a cancellable, single-threaded iterative-deepening alpha-beta search with quiescence, principal variations, repetition and draw handling, a persistent fixed-size transposition table, tapered positional evaluation, a bounded attacking personality, and volatility-aware soft/hard clock management. It is UCI-playable and exposes a reproducibly gated `Aggression` profile from 0 to 100.

## Goals

Jakgro will favor initiative and practical winning chances without replacing chess correctness with random sacrifices. The intended style will come from search and evaluation terms that value:

- pressure against the enemy king;
- forcing moves and sustained initiative;
- active mobility and spatial control;
- dangerous passed pawns;
- tactically justified material investment; and
- bounded draw aversion when a position offers winning chances.

Legality, tactical soundness, and reproducible testing remain hard constraints. The engine and UCI APIs bound `Aggression` from 0 to 100 and default to the tuned attacking profile at 100. Fixed-node fixtures gate both endpoints so style changes remain deliberate and reviewable.

## Requirements

- Rust 1.85 or newer
- Python 3 for the measurement helpers
- `cutechess-cli` only when running paired match measurements

## Build and test

```sh
cargo build --release --locked
cargo test --locked
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

The release executable is written to `target/release/jakgro`.

## UCI usage

Run the engine directly:

```sh
cargo run --release --locked
```

A minimal session looks like this. Send `quit` only after the engine has returned `bestmove`:

```text
uci
isready
position startpos moves e2e4 e7e5
go depth 4
info depth 1 score cp 0 nodes 29 time 1 nps 29000 pv a2a3
...
bestmove a2a3 ponder a7a5
quit
```

Node counts, timing, principal variations, and selected moves vary with the position and search limit.

Jakgro currently handles these GUI commands:

- `uci`
- `debug on|off`
- `isready`
- `setoption name Hash value <MiB>`, `setoption name Clear Hash`, `setoption name Aggression value <0..100>`, and `setoption name Move Overhead value <milliseconds>`
- `ucinewgame`
- `position startpos ...`
- `position fen <six FEN fields> ...`
- `go` with `searchmoves`, `ponder`, clocks, increments, `movestogo`, depth, nodes, mate, `movetime`, or `infinite`
- `stop`
- `ponderhit`
- `quit`

Each completed iterative-deepening pass emits an `info` line containing depth, a centipawn or mate score, cumulative nodes, elapsed milliseconds, nodes per second, and the principal variation. The final line is `bestmove`, with a ponder move when the principal variation contains a reply.

Search runs on a worker while the protocol loop remains responsive. `stop` publishes the latest completed iteration or a legal fallback, `isready` responds during search, replacement position/search commands suppress stale results, and `quit` or end of input cancels without emitting a final move. Infinite-search results are withheld until `stop`; ponder results are withheld until `ponderhit` or `stop`.

Malformed and unknown commands do not terminate the process. With debug mode enabled, ignored commands are reported using `info string` messages.

To use Jakgro from a chess GUI, build the release binary and configure the GUI to launch the resulting `target/release/jakgro` executable as a UCI engine.

## Reproducible style and match measurement

The style gate drives the public UCI interface at fixed node budgets and compares Aggression 0 and 100 against `tests/data/personality.epd`:

```sh
cargo build --release --locked
python3 tools/measure_style.py --engine target/release/jakgro --check
```

The command prints CSV observations containing each profile's move, score, completed depth, node count, and gate status. CI runs the same command, while `cargo test --test aggression_profile` independently checks both profiles through the engine API.

For paired self-play, install `cutechess-cli` and run:

```sh
python3 tools/run_match.py \
  --engine target/release/jakgro \
  --games 96 \
  --nodes 50000 \
  --pgn artifacts/aggression-match.pgn
```

The match runner compares Aggression 100 with Aggression 0 using reversed colors, one unique sequential EPD opening per pair, one concurrent game, fixed nodes per move, and explicit draw, resignation, and maximum-move rules. Each completed run writes an atomic JSON sidecar containing binary and opening hashes, the cutechess version, the exact command, settings, timing, return status, game count, and PGN hash. Pass `--baseline-engine` to compare against another binary, `--openings` to supply a larger suite, `--manifest` to choose the sidecar path, or `--dry-run` to inspect the command without launching a match.

## Current search and protocol limitations

- Only standard chess is supported; Chess960 is deferred.
- Static evaluation tapers material, activity, mobility, bishop-pair, pawn-structure, passed-pawn, and king-shelter features between middlegame and endgame. A bounded profile adds initiative, king-zone pressure, pawn storms, favorable threats, space, passed-pawn urgency, and a small root complexity preference.
- Search is single-threaded internally and uses one worker per active UCI search.
- Every child still clones the `cozy-chess` board; there is no make/unmake layer yet. A persistent fixed-size transposition table reuses exact and bounded search results.
- Move ordering combines hash and previous-PV moves, promotions, tactical capture values, killers, and history scores. Principal-variation search and aspiration windows reduce repeated work; null-move pruning, late-move reductions, and a make/unmake board layer are still deferred.
- A `go` command without an effective time, node, depth, mate, infinite, or ponder limit defaults to depth four so accidental limit-free searches terminate.
- Clock-managed searches use a normal soft budget and a reserved hard limit. Stable iterations stop at the soft limit; best-move changes or large score swings can spend toward the hard limit. `Move Overhead` reserves 0–5000 ms for GUI and operating-system latency, while an explicit `movetime` remains fixed.
- Repetition history is retained for moves supplied after `position`; a standalone FEN cannot describe occurrences before that FEN.
- No `Threads` or `MultiPV` UCI options are advertised.

## Architecture

- `src/engine/position.rs` isolates legal position and UCI-move handling from the board library and retains normalized repetition hashes.
- `src/engine/evaluation.rs` and `src/engine/evaluation/` contain bounded tapered scoring, feature extraction, trace data, weights, and mate-score constants.
- `src/engine/search/algorithm.rs` implements iterative deepening, negamax alpha-beta, quiescence, deterministic move ordering, draw detection, and principal-variation construction.
- `src/engine/search/transposition.rs` owns the fixed-size, generation-aged search cache and mate-score normalization.
- `src/engine/search/control.rs` provides shared cancellation and updateable soft/hard deadlines.
- `src/engine/search/time.rs` converts UCI clock fields and move overhead into normal and emergency budgets.
- `src/uci/session.rs` owns the serialized protocol event loop.
- `src/uci/search_worker.rs` isolates search threads, generation IDs, pondering, and stale-result suppression.
- `tools/measure_style.py` reports and gates fixed-node choices across Aggression profiles.
- `tools/run_match.py` builds deterministic paired fixed-node matches for `cutechess-cli`.
- `src/main.rs` is a thin adapter that reserves stdout exclusively for UCI traffic.

## Milestone status

The initial protocol and search foundations now include:

- legal standard-chess position handling with special-move coverage;
- normalized repetition history and terminal draw detection;
- iterative-deepening alpha-beta with bounded quiescence;
- cancellation, depth, node, mate, soft/hard clock, `movetime`, infinite, and ponder control;
- principal-variation and UCI progress reporting;
- a persistent transposition table with safe draw-state handling and UCI `Hash` controls;
- volatility-aware time allocation with a persistent UCI `Move Overhead` control;
- a bounded, color-symmetric attacking profile with isolated style weights and UCI `Aggression` control;
- fixed-node style gates and deterministic paired-match tooling; and
- asynchronous `stop`, `ponderhit`, replacement-search, EOF, and shutdown behavior.

## Roadmap

1. **Search efficiency and repeatability**
   - Add null-move pruning, late-move reductions, and a make/unmake board layer.
   - Expand deterministic search and performance benchmarks.
2. **Aggressive evaluation**
   - Expand the gated suite with compensation, pawn-race, and defensive-resource positions.
   - Measure tactical soundness separately from style before adding sacrifice-specific terms.
3. **Time and protocol refinement**
   - Tune soft/hard budget ratios and volatility thresholds through timed matches.
   - Evaluate thread-safe shared search structures before advertising a `Threads` option.
   - Expand ponder, mate-limit, and malformed-command regression suites.
4. **Tuning and match testing**
   - Track Elo, decisive-game rate, color balance, and sacrifice quality against tagged versions.
   - Tune explicit style weights only when fixture gates and paired matches agree.

## License

No license has been selected yet.
