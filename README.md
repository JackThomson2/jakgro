# Jakgro

Jakgro is a Rust chess engine aimed at playing aggressive, tactical, and interesting chess while remaining compatible with the Universal Chess Interface (UCI).

> **Current status:** Jakgro now runs a cancellable, single-threaded iterative-deepening alpha-beta search with quiescence, principal variations, repetition and draw handling, a persistent fixed-size transposition table, tapered positional evaluation, a bounded attacking personality, and basic clock management. It is UCI-playable; personality tuning and UCI exposure are still under development.

## Goals

Jakgro will favor initiative and practical winning chances without replacing chess correctness with random sacrifices. The intended style will come from search and evaluation terms that value:

- pressure against the enemy king;
- forcing moves and sustained initiative;
- active mobility and spatial control;
- dangerous passed pawns;
- tactically justified material investment; and
- bounded draw aversion when a position offers winning chances.

Legality, tactical soundness, and reproducible testing remain hard constraints. The engine API bounds the attacking profile from 0 to 100; no UCI `Aggression` option is advertised until its behavior is tuned and regression-tested.

## Requirements

- Rust 1.85 or newer

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
- `setoption name Hash value <MiB>` and `setoption name Clear Hash`
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

### Current search and protocol limitations

- Only standard chess is supported; Chess960 is deferred.
- Static evaluation tapers material, activity, mobility, bishop-pair, pawn-structure, passed-pawn, and king-shelter features between middlegame and endgame. A bounded profile adds initiative, king-zone pressure, pawn storms, favorable threats, space, passed-pawn urgency, and a small root complexity preference.
- Search is single-threaded internally and uses one worker per active UCI search.
- Every child still clones the `cozy-chess` board; there is no make/unmake layer yet. A persistent fixed-size transposition table reuses exact and bounded search results.
- Move ordering consists of the previous principal variation, promotions, and MVV-LVA-style captures; there are no killer, history, hash-move, or aspiration-window heuristics.
- A `go` command without an effective time, node, depth, mate, infinite, or ponder limit defaults to depth four so accidental limit-free searches terminate.
- Clock allocation is intentionally basic and has no configurable move-overhead option.
- Repetition history is retained for moves supplied after `position`; a standalone FEN cannot describe occurrences before that FEN.
- No `Threads`, `MultiPV`, or aggression-related UCI options are advertised.

## Architecture

- `src/engine/position.rs` isolates legal position and UCI-move handling from the board library and retains normalized repetition hashes.
- `src/engine/evaluation.rs` and `src/engine/evaluation/` contain bounded tapered scoring, feature extraction, trace data, weights, and mate-score constants.
- `src/engine/search/algorithm.rs` implements iterative deepening, negamax alpha-beta, quiescence, deterministic move ordering, draw detection, and principal-variation construction.
- `src/engine/search/transposition.rs` owns the fixed-size, generation-aged search cache and mate-score normalization.
- `src/engine/search/control.rs` provides shared cancellation and updateable deadlines.
- `src/engine/search/time.rs` converts UCI clock fields into a basic move budget.
- `src/uci/session.rs` owns the serialized protocol event loop.
- `src/uci/search_worker.rs` isolates search threads, generation IDs, pondering, and stale-result suppression.
- `src/main.rs` is a thin adapter that reserves stdout exclusively for UCI traffic.

## Milestone status

The initial protocol and search foundations now include:

- legal standard-chess position handling with special-move coverage;
- normalized repetition history and terminal draw detection;
- iterative-deepening alpha-beta with bounded quiescence;
- cancellation, depth, node, mate, clock, `movetime`, infinite, and ponder control;
- principal-variation and UCI progress reporting;
- a persistent transposition table with safe draw-state handling and UCI `Hash` controls;
- a bounded, color-symmetric attacking profile with isolated style weights; and
- asynchronous `stop`, `ponderhit`, replacement-search, EOF, and shutdown behavior.

## Roadmap

1. **Search efficiency and repeatability**
   - Add hash-move, killer, history, and improved tactical ordering.
   - Add aspiration windows and deterministic search benchmarks.
2. **Aggressive evaluation**
   - Tune the bounded initiative, king-pressure, space, threat, and passer-urgency terms against stable fixtures.
   - Measure tactical soundness separately from style before expanding compensation terms.
3. **Time and protocol refinement**
   - Add configurable move overhead and more conservative panic-time handling.
   - Evaluate thread-safe shared search structures before advertising a `Threads` option.
   - Expand ponder, mate-limit, and malformed-command regression suites.
4. **Tuning and match testing**
   - Tune against curated tactical and attacking positions.
   - Measure Elo, decisive-game rate, sacrifice quality, and regressions against stable engine versions.
   - Expose aggression-related UCI options only after their behavior is measurable and tested.

## License

No license has been selected yet.
