# Jakgro

Jakgro is a Rust chess engine aimed at playing aggressive, tactical, and interesting chess while remaining compatible with the Universal Chess Interface (UCI).

> **Current status:** the engine understands positions, generates legal standard-chess moves, and can be loaded by a UCI client. Its search is intentionally only a deterministic legal-move baseline; it does not yet play strong chess or use the supplied time controls.

## Goals

Jakgro will favor initiative and practical winning chances without replacing chess correctness with random sacrifices. The intended style will come from search and evaluation terms that value:

- pressure against the enemy king;
- forcing moves and sustained initiative;
- active mobility and spatial control;
- dangerous passed pawns;
- tactically justified material investment; and
- bounded draw aversion when a position offers winning chances.

Legality, tactical soundness, and reproducible testing remain hard constraints. No UCI `Aggression` option is advertised until it changes actual engine behavior.

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

A minimal session looks like this:

```text
uci
isready
position startpos moves e2e4 e7e5
go depth 1
quit
```

Jakgro currently handles these GUI commands:

- `uci`
- `debug on|off`
- `isready`
- `setoption` (recognized, but no options are advertised yet)
- `ucinewgame`
- `position startpos ...`
- `position fen <six FEN fields> ...`
- `go` with standard search-limit fields
- `stop`
- `ponderhit`
- `quit`

Malformed and unknown commands do not terminate the process. With debug mode enabled, ignored commands are reported using `info string` messages.

To use Jakgro from a chess GUI, build the release binary and configure the GUI to launch the resulting `target/release/jakgro` executable as a UCI engine.

### Current protocol limitations

- Only standard chess is supported; Chess960 is deferred.
- `go` returns a deterministic legal move immediately. Parsed depth, node, clock, and pondering limits are scaffolding for the real searcher.
- Because the baseline search completes immediately, `stop` and `ponderhit` currently have no active search to control.
- No `info depth`, score, node, or principal-variation output is produced yet.

## Architecture

- `src/engine/position.rs` isolates legal position and UCI-move handling from the board library.
- `src/engine/search.rs` defines search limits and the disposable baseline move selector.
- `src/uci/` parses and runs protocol sessions independently from standard input and output.
- `src/main.rs` is a thin adapter that reserves stdout exclusively for UCI traffic.

## Roadmap

1. **Correctness and regression fixtures**
   - Add perft positions and deeper move-generation integration tests.
   - Track repetition, the fifty-move rule, and game history required by search.
2. **Cancellable search**
   - Add iterative deepening, alpha-beta pruning, quiescence search, and principal-variation reporting.
   - Move search behind a worker that obeys `stop`, pondering, and time controls.
3. **Search efficiency**
   - Add a transposition table, aspiration windows, killer/history heuristics, and tactical move ordering.
   - Add deterministic benchmarks for nodes per second and search regressions.
4. **Aggressive evaluation**
   - Build conventional material and king-safety foundations first.
   - Add initiative, king-zone pressure, mobility, space, passed-pawn, and compensation terms.
   - Keep style weights explicit so tactical strength and aggression can be measured separately.
5. **Tuning and match testing**
   - Tune against curated tactical and attacking positions.
   - Measure Elo, decisive-game rate, sacrifice quality, and regressions against stable engine versions.
   - Expose UCI options only after their behavior is covered by tests.

## License

No license has been selected yet.
