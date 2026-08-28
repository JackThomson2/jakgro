# Jakgro

Jakgro is a Rust chess engine aimed at playing aggressive, tactical, and interesting chess while remaining compatible with the Universal Chess Interface (UCI).

> **Current status:** Jakgro runs a cancellable iterative-deepening alpha-beta search with quiescence, principal variations, repetition and draw handling, a lock-free fixed-size transposition table consulted in quiescence as well as in the main search, optional lazy SMP across a configurable thread count, tapered positional evaluation with piece-square tables and a cached pawn structure, a bounded attacking personality, and volatility-aware soft/hard clock management. It is UCI-playable and exposes a reproducibly gated `Aggression` profile from 0 to 100. Two measured series are recorded: [`docs/tuning/strength-series.md`](docs/tuning/strength-series.md) at +108 Elo at the default profile, and [`docs/tuning/strength-series-two.md`](docs/tuning/strength-series-two.md) at a further +65 Elo on top of it. Parallel search defaults to one thread and carries no measured Elo claim yet; see [`docs/tuning/lazy-smp.md`](docs/tuning/lazy-smp.md).

## Goals

Jakgro will favor initiative and practical winning chances without replacing chess correctness with random sacrifices. The intended style will come from search and evaluation terms that value:

- pressure against the enemy king;
- forcing moves and sustained initiative;
- active mobility and spatial control;
- dangerous passed pawns;
- tactically justified material investment; and
- bounded draw aversion when a position offers winning chances.

Legality, tactical soundness, and reproducible testing remain hard constraints. The engine and UCI APIs bound `Aggression` from 0 to 100 and default to the accepted attacking profile at 75; profile 100 remains available as the wilder endpoint. Fixed-node fixtures gate the objective, default, and maximum profiles so style changes remain deliberate and reviewable.

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

The release executable is written to `target/release/jakgro`. Release builds use fat link-time optimization and a single codegen unit, which roughly doubles link time in exchange for a measurably faster search. The benchmark profile inherits those optimization settings, and Cargo always builds benchmarks with unwinding so their assertions still report failures.

Jakgro resolves `cozy-chess` from the [`board-state-save-restore` branch](https://github.com/JackThomson2/cozy-chess/tree/board-state-save-restore). `Cargo.lock` pins the exact Git revision for reproducible builds. The UCI executable and standalone search benchmark select mimalloc globally; the reusable library does not force an allocator on downstream binaries.

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
- `setoption name Hash value <MiB>`, `setoption name Clear Hash`, `setoption name Threads value <1..128>`, `setoption name Aggression value <0..100>`, and `setoption name Move Overhead value <milliseconds>`
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
- `100` uses a 45-centipawn ordinary margin, tightens winning conversions to 20, and reserves the absolute 120-centipawn ceiling for verified investments or already-losing positions where complications are valuable.

A sacrifice preference requires a full opponent reply, legal recapture settlement, settled attacking compensation, retained king safety, and a checking resource. Truncated exchanges, declined offers, and immediately recovered material receive no sacrifice preference. At high aggression, an eligible live line also outranks immediate repetition, terminal draws, and balanced queen or rook exchanges that do not increase the attack.

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
  --markdown artifacts/aggression-match.summary.md \
  --min-elo-lower-bound 0
```

The analyzer verifies the PGN hash and completed game count against the manifest, requires consecutive color-reversed opening pairs, and reports W/D/L, score, color balance, pair outcomes, terminations, average length, SAN-derived checks, captures, promotions, forcing-move rates, and a conservative pair-aware 95% score and Elo bound. `--min-elo-lower-bound` turns that bound into a strict acceptance gate. The style rates are descriptive proxies rather than move-quality judgments, and the interval is not an SPRT result. The historical Aggression 100 versus 0 baseline is recorded in [`docs/tuning/aggression-100-vs-0.md`](docs/tuning/aggression-100-vs-0.md); the current old-versus-new result is recorded in [`docs/tuning/verified-aggression-elo.md`](docs/tuning/verified-aggression-elo.md); the accepted null-only search result is recorded in [`docs/tuning/verified-null-move.md`](docs/tuning/verified-null-move.md); the measured selective-search result is recorded in [`docs/tuning/selective-search-strength.md`](docs/tuning/selective-search-strength.md); and the confirmed strength series is recorded in [`docs/tuning/strength-series.md`](docs/tuning/strength-series.md).

### Sequential testing without cutechess-cli

`tools/run_sprt.py` drives the `selfplay` arbiter, which plays paired
colour-reversed games over real UCI pipes and needs no external match runner:

```sh
cargo build --release --locked --bin jakgro --bin selfplay
python3 tools/run_sprt.py \
  --engine target/release/jakgro \
  --baseline-engine artifacts/baseline-jakgro \
  --candidate-aggression 75 --baseline-aggression 75 \
  --games 4096 --movetime-ms 50 --concurrency 44 \
  --elo0 0 --elo1 10 \
  --openings docs/tuning/data/selective-search-confirmation.epd \
  --pgn artifacts/sprt.pgn
```

The arbiter adjudicates with the same draw, resignation, and move-limit rules the
`cutechess-cli` invocations use, and treats illegal moves, protocol timeouts,
disconnections, and clock overruns as forfeits attributed to one engine rather
than as draws. It accepts `--nodes`, `--movetime-ms`, or a `--time-control` such
as `1.0+0.01`, in which case it maintains both clocks and charges each search
against them. The harness parses the resulting PGN back through
`analyze_match.py` rather than trusting the arbiter's own tally, accumulates
colour-reversed pair scores into pentanomial buckets, and reports a paired normal
interval alongside a Wald sequential test. A match with any recorded fault is
reported as failed and yields no verdict. Because the pair variance is estimated
directly, the interval is considerably tighter than the Hoeffding bound
`analyze_match.py` reports at the same game count. Its PGN remains compatible with
`analyze_match.py` and `gate_strength_personality.py`.

Compare tree size, throughput, and completed depth before running matches with:

```sh
python3 tools/measure_search_efficiency.py \
  --engine target/release/jakgro \
  --baseline-engine artifacts/baseline-jakgro \
  --depth 4 \
  --samples 7 \
  --move-time-ms 500 \
  --summary-json artifacts/search-efficiency.json \
  --check
```

The paired runner alternates the two binaries over the frozen performance suite,
reports fixed-depth node reduction, median fixed-node NPS, and fixed-time depth,
and binds the JSON to exact binary and suite hashes. See
[`docs/tuning/search-performance.md`](docs/tuning/search-performance.md) for the
measurement protocol and interpretation rules.

`tools/gate_strength_personality.py` then binds objective old-versus-new, accepted-profile old-versus-new, candidate and baseline default-versus-objective, fixed-position style, objective-loss, sacrifice, and efficiency artifacts to one contract. It uses Elo confidence bounds for old-versus-new channels, enforces an absolute floor on the candidate's personality cost, compares that cost with the baseline binary, and fails on binary-identity mismatches. The accepted default profile and its exploratory match evidence are recorded in [`docs/tuning/default-aggression-75.md`](docs/tuning/default-aggression-75.md); wide intervals explicitly prevent treating the point estimates as publishable Elo claims.

## Current search and protocol limitations

- Only standard chess is supported; Chess960 is deferred.
- Static evaluation tapers material, tuned piece-square placement, tempo, activity, mobility, bishop-pair, pawn-structure, passed-pawn, and king-shelter features between middlegame and endgame. Search scores and transposition bounds remain personality-neutral; aggression instead controls tactical search policy and root interest in coordinated king attacks, supported threats, open attacking lines, and pawn breaks.
- Higher aggression spends additional search effort on checks and forcing continuations. Root personality work threshold-probes diverse alternatives, fully verifies at most two inside a deterministic node budget, and keeps only completed verification when that local budget expires. Ordinary choices use a 30-centipawn cap, winning conversions use 20, non-negative objective results cannot cross below zero, and only independently verified sacrifices may use the absolute 120-centipawn ceiling.
- Search runs on a configurable number of threads through lazy SMP. One thread, the default, is deterministic and is what every fixed-node fixture, aggression gate, and recorded series measures. More than one thread shares the transposition table between searchers and is deliberately not reproducible move for move, because the tree the helpers explore depends on how their timing interleaves.
- Every child clones the `cozy-chess` board. A make/unmake layer was implemented and measured for an earlier series and rejected: `size_of::<Board>()` equals `size_of::<BoardState>()`, so a snapshot costs as much as the copy it avoids. A persistent fixed-size transposition table reuses exact and bounded search results, and quiescence consults it as well, which matters because quiescence is roughly 97% of all nodes. Each entry packs its payload into one machine word beside a verification word, so a bucket is one cache line and several searchers can read and write it without locking.
- Move ordering combines hash and previous-PV moves, promotions, swap-list static-exchange values, killers, signed butterfly history, and agreement-bounded continuation history. Principal-variation search, aspiration windows, table-driven late-move reductions, depth-indexed move-count pruning, always-verified null-move pruning, and conservative quiescence pruning reduce repeated work. Move-count pruning exempts checks, castling, king-zone moves, killers, hash and PV moves, moves with positive history, and — at non-zero aggression — attacking pawn pushes, so the attacking profile keeps its forcing continuations. Null pruning is disabled in checks, PV and mate windows, rule-fifty boundaries, pawn-only and single-minor endings, and synthetic or verification searches; every fail-high is verified from the original legal board.
- A `go` command without an effective time, node, depth, mate, infinite, or ponder limit defaults to depth four so accidental limit-free searches terminate.
- Clock-managed searches use a normal soft budget and a reserved hard limit. Stable iterations stop at the soft limit; best-move changes or large score swings can spend toward the hard limit. `Move Overhead` reserves 0–5000 ms for GUI and operating-system latency, while an explicit `movetime` remains fixed.
- Repetition history is retained for moves supplied after `position`; a standalone FEN cannot describe occurrences before that FEN.
- No `MultiPV` UCI option is advertised. `Threads` is advertised and defaults to one.

## Architecture

- `src/engine/position.rs` isolates legal position and UCI-move handling from the board library and retains normalized repetition hashes.
- `src/engine/evaluation.rs` and `src/engine/evaluation/` contain bounded tapered scoring, tuned piece-square tables, feature extraction, legal exchange settlement, mover-relative tactical snapshots, trace data, weights, and mate-score constants.
- `src/engine/search/algorithm.rs` implements iterative deepening, negamax alpha-beta, quiescence, deterministic move ordering, draw detection, always-verified null-move pruning, tactical-aware late-move reductions, sacrifice profiling after best defense, bounded root-risk selection, null telemetry, principal-variation construction, and the lazy SMP driver that runs one main searcher alongside diversified helpers.
- `src/engine/search/see.rs` performs allocation-free swap-list static-exchange analysis for ordering and conservative quiescence pruning; exact legal settlement for sacrifice verification lives in `src/engine/evaluation/tactics.rs`.
- `src/engine/search/transposition.rs` owns the fixed-size, generation-aged search cache, its lock-free atomic slots, and mate-score normalization.
- `src/engine/search/control.rs` provides shared cancellation and updateable soft/hard deadlines.
- `src/engine/search/time.rs` converts UCI clock fields and move overhead into normal and emergency budgets.
- `src/uci/session.rs` owns the serialized protocol event loop.
- `src/uci/search_worker.rs` isolates the search worker thread, generation IDs, pondering, and stale-result suppression; the searchers a parallel search spawns live below it inside the search itself.
- `tools/measure_style.py` and `tools/measure_acceptance.py` report fixed-node choices and objective root loss across Aggression profiles.
- `tools/measure_search_efficiency.py` compares old/new fixed-depth nodes, fixed-node throughput, and fixed-time completed depth.
- `tools/run_match.py` builds deterministic paired fixed-node matches for `cutechess-cli`, optionally at a chosen `Threads` count.
- `src/bin/selfplay.rs` is a self-contained paired-match arbiter for hosts without `cutechess-cli`.
- `tools/run_sprt.py` evaluates a paired match with pentanomial pair statistics and a Wald sequential test.
- `tools/analyze_match.py` validates paired PGNs and reports strength, confidence, color balance, and descriptive forcing-play rates.
- `tools/gate_strength_personality.py` binds all strength, style, acceptance, and efficiency channels to exact binary hashes and confidence floors.
- `src/main.rs` is a thin adapter that reserves stdout exclusively for UCI traffic.

## Milestone status

The initial protocol and search foundations now include:

- legal standard-chess position handling with special-move coverage;
- normalized repetition history and terminal draw detection;
- iterative-deepening alpha-beta with bounded quiescence;
- cancellation, depth, node, mate, soft/hard clock, `movetime`, infinite, and ponder control;
- principal-variation and UCI progress reporting;
- a persistent transposition table with safe draw-state handling and UCI `Hash` controls;
- lazy SMP over a lock-free shared table with a deterministic single-threaded default and UCI `Threads` control;
- volatility-aware time allocation with a persistent UCI `Move Overhead` control;
- a bounded, color-symmetric attacking profile with isolated style weights, settled best-defense sacrifice verification, contextual root-risk guards, draw and simplification aversion, and UCI `Aggression` control;
- threshold-probed root personality work, table-driven late-move reductions, move-count pruning with forcing-move exemptions, swap-list static-exchange ordering/pruning, and always-verified null pruning;
- fixed-node old-versus-new style gates, conservative Elo acceptance, deterministic paired-match tooling, in-repo sequential testing, and null-on/null-off telemetry; and
- asynchronous `stop`, `ponderhit`, replacement-search, EOF, and shutdown behavior.

## Roadmap

1. **Search efficiency and repeatability**
   - Tune the bounded continuation-history signal against held-out fixed-depth positions. A make/unmake layer was measured and rejected, since a board snapshot is the same size as the board it replaces.
   - Retry singular extensions with a cheaper probe. A full implementation measured neutral because the exclusion search re-expands a large quiescence subtree; capping its quiescence depth or reusing the parent's move list is the obvious next attempt. Internal iterative reduction also measured neutral to negative and is recorded in [`docs/tuning/strength-series-two.md`](docs/tuning/strength-series-two.md).
   - Repeat verified-null and old/new differential benchmarks across more positions and platforms.
2. **Aggressive evaluation**
   - Retry king safety from safe checks and per-square attack units. A non-linear attacker-count term was measured at -14 Elo and rejected; see [`docs/tuning/strength-series.md`](docs/tuning/strength-series.md).
   - Add rook-file, outpost, and endgame-scaling terms, which the evaluation still lacks entirely.
   - Add held-out pawn-gambit, clearance-sacrifice, and exchange-sacrifice positions without weakening the anti-sacrifice controls.
   - Measure whether verified sacrifices survive deeper searches and human PGN review before changing the 120-centipawn hard guard.
3. **Time and protocol refinement**
   - Scale the soft clock by best-move stability and root node effort, then confirm through timed matches rather than fixed-move-time ones.
   - Report at least one `info` line for every completed search. Some positions currently return a `bestmove` alone, which the measurement tools cannot read.
   - Measure lazy SMP strength at equal time control and record whether the scaling in searched nodes converts into Elo, following [`docs/tuning/lazy-smp.md`](docs/tuning/lazy-smp.md).
   - Expand ponder, mate-limit, and malformed-command regression suites.
4. **Tuning and match testing**
   - Reduce the measured strength gap between Aggression 100 and Aggression 0 without surrendering the forcing-play and safety gates.
   - Track old-versus-new Elo, decisive-game rate, color balance, independently reviewed sacrifice frequency, and manual sacrifice quality against tagged versions.
   - Tune explicit style weights only when frozen fixture gates and paired matches agree.
   - Confirm the series result at a longer time control than 50 ms per move.

## License

No license has been selected yet.
