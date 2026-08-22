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
- network access to fetch the optimized [`cozy-chess` fork](https://github.com/JackThomson2/cozy-chess/tree/board-state-save-restore) on the first build
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

Jakgro resolves `cozy-chess` from the [`board-state-save-restore` branch](https://github.com/JackThomson2/cozy-chess/tree/board-state-save-restore). `Cargo.lock` pins the exact Git revision for reproducible builds.

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

### Aggression semantics

`Aggression` remains deterministic, but it now changes both what Jakgro values and how much searched score it will exchange for an interesting root move:

- `0` disables style evaluation, retains the conventional forcing-search budgets, and uses the ordinary alpha-beta best move;
- intermediate values gradually add coordinated attack terms, tactical search effort, and a nonlinear root-choice margin; and
- `100` uses the ordinary 120-centipawn margin for interesting alternatives, but may widen it to about 220 centipawns for a verified pawn investment, 380 for a verified piece or exchange investment, or a hard 450 ceiling when already clearly worse. Speculative risk is capped near 200 centipawns when clearly winning.

A larger margin requires a full opponent reply, legal recapture settlement, multiple attacking compensation signals, retained king safety, and at least one checking resource. Unverified offers and apparent sacrifices that are immediately recovered use the ordinary margin. At high aggression, an eligible live line also outranks immediate repetition, terminal draws, and balanced queen or rook exchanges that do not increase the attack.

Mate scores always outrank centipawn style preferences, and clearly forced defenses fall outside the bounded margins. Jakgro reports the selected move's actual searched score and principal variation over UCI; it never adds the entertainment score to the reported chess score. High settings can deliberately play weaker chess, which is the intended tradeoff rather than a strength claim.

## Reproducible style and match measurement

The style gate drives the public UCI interface at fixed node budgets and compares Aggression 0 and 100 against `tests/data/personality.epd`:

```sh
cargo build --release --locked
python3 tools/measure_style.py \
  --engine target/release/jakgro \
  --check \
  --summary-json artifacts/style-summary.json
```

The command prints CSV observations containing each fixture category, profile move, acceptable move set, score, completed depth, node count, and gate status. The optional JSON summary reports hit rates by category and profile. CI runs the same endpoint checks, while `cargo test --test aggression_profile` independently checks both profiles and the mandatory tactical and defensive controls through the engine API.

For paired self-play, install `cutechess-cli` and run:

```sh
python3 tools/run_match.py \
  --engine target/release/jakgro \
  --games 96 \
  --nodes 50000 \
  --pgn artifacts/aggression-match.pgn
```

The match runner compares Aggression 100 with Aggression 0 using reversed colors, one unique sequential EPD opening per pair, one concurrent game, fixed nodes per move, and explicit draw, resignation, and maximum-move rules. Each completed run writes an atomic JSON sidecar containing binary and opening hashes, the cutechess version, the exact command, settings, timing, return status, game count, and PGN hash. Pass `--baseline-engine` to compare against another binary, `--openings` to supply a larger suite, `--manifest` to choose the sidecar path, or `--dry-run` to inspect the command without launching a match.

Summarize a completed paired match with:

```sh
python3 tools/analyze_match.py \
  --pgn artifacts/aggression-match.pgn \
  --json artifacts/aggression-match.summary.json \
  --markdown artifacts/aggression-match.summary.md
```

The analyzer verifies the PGN hash and completed game count against the manifest, requires consecutive color-reversed opening pairs, and reports W/D/L, score, color balance, pair outcomes, terminations, average length, SAN-derived checks, captures, promotions, forcing-move rates, and an approximate pair-aware 95% score and Elo interval. The style rates are descriptive proxies rather than move-quality judgments, and the interval is not an SPRT result. The first reproducible Aggression 100 versus 0 baseline is recorded in [`docs/tuning/aggression-100-vs-0.md`](docs/tuning/aggression-100-vs-0.md); the wilder profile's validation protocol is recorded in [`docs/tuning/wilder-aggression-profile.md`](docs/tuning/wilder-aggression-profile.md).

## Current search and protocol limitations

- Only standard chess is supported; Chess960 is deferred.
- Static evaluation tapers material, activity, mobility, bishop-pair, pawn-structure, passed-pawn, and king-shelter features between middlegame and endgame. High aggression adds coordinated king-zone attackers, supported threats, open attacking lines, pawn breaks, and bounded compensation for material risk.
- High aggression spends additional search effort on checks and forcing continuations, then fully verifies up to six high-interest root alternatives. This deliberately reduces conventional search depth and can select a move up to the documented root margin below the alpha-beta best score.
- Search is single-threaded internally and uses one worker per active UCI search.
- Every child still clones the `cozy-chess` board; there is no make/unmake layer yet. A persistent fixed-size transposition table reuses exact and bounded search results.
- Move ordering combines hash and previous-PV moves, promotions, tactical capture values, killers, and history scores. Principal-variation search and aspiration windows reduce repeated work; null-move pruning, late-move reductions, and a make/unmake board layer are still deferred.
- A `go` command without an effective time, node, depth, mate, infinite, or ponder limit defaults to depth four so accidental limit-free searches terminate.
- Clock-managed searches use a normal soft budget and a reserved hard limit. Stable iterations stop at the soft limit; best-move changes or large score swings can spend toward the hard limit. `Move Overhead` reserves 0–5000 ms for GUI and operating-system latency, while an explicit `movetime` remains fixed.
- Repetition history is retained for moves supplied after `position`; a standalone FEN cannot describe occurrences before that FEN.
- No `Threads` or `MultiPV` UCI options are advertised.

## Architecture

- `src/engine/position.rs` isolates legal position and UCI-move handling from the board library and retains normalized repetition hashes.
- `src/engine/evaluation.rs` and `src/engine/evaluation/` contain bounded tapered scoring, feature extraction, legal exchange settlement, mover-relative tactical snapshots, trace data, weights, and mate-score constants.
- `src/engine/search/algorithm.rs` implements iterative deepening, negamax alpha-beta, quiescence, deterministic move ordering, draw detection, sacrifice profiling after best defense, bounded root-risk selection, and principal-variation construction.
- `src/engine/search/transposition.rs` owns the fixed-size, generation-aged search cache and mate-score normalization.
- `src/engine/search/control.rs` provides shared cancellation and updateable soft/hard deadlines.
- `src/engine/search/time.rs` converts UCI clock fields and move overhead into normal and emergency budgets.
- `src/uci/session.rs` owns the serialized protocol event loop.
- `src/uci/search_worker.rs` isolates search threads, generation IDs, pondering, and stale-result suppression.
- `tools/measure_style.py` reports and gates fixed-node choices across Aggression profiles.
- `tools/run_match.py` builds deterministic paired fixed-node matches for `cutechess-cli`.
- `tools/analyze_match.py` validates paired PGNs and reports strength, confidence, color balance, and descriptive forcing-play rates.
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
- a bounded, color-symmetric attacking profile with isolated style weights, best-defense sacrifice verification, draw and simplification aversion, and UCI `Aggression` control;
- fixed-node style gates and deterministic paired-match tooling; and
- asynchronous `stop`, `ponderhit`, replacement-search, EOF, and shutdown behavior.

## Roadmap

1. **Search efficiency and repeatability**
   - Add null-move pruning, late-move reductions, and a make/unmake board layer.
   - Expand deterministic search and performance benchmarks.
2. **Aggressive evaluation**
   - Add held-out pawn-gambit and exchange-sacrifice positions without weakening the anti-sacrifice controls.
   - Measure whether verified sacrifices survive deeper searches and human PGN review before widening the 450-centipawn hard ceiling.
3. **Time and protocol refinement**
   - Tune soft/hard budget ratios and volatility thresholds through timed matches.
   - Evaluate thread-safe shared search structures before advertising a `Threads` option.
   - Expand ponder, mate-limit, and malformed-command regression suites.
4. **Tuning and match testing**
   - Test Aggression 75 next; the initial Aggression 100 baseline scored 44.27% against Aggression 0 with a confidence interval crossing 50%.
   - Track Elo, decisive-game rate, color balance, verified-sacrifice frequency, and manual sacrifice quality against tagged versions.
   - Tune explicit style weights only when fixture gates and paired matches agree.

## License

No license has been selected yet.
