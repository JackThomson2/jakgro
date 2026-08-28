# Lazy SMP

Jakgro searches on a configurable number of threads. `Threads` defaults to one
and is bounded from one to 128. No Elo claim is recorded here: this document
describes the design, states what is and is not reproducible, and specifies the
protocol that a strength measurement must follow.

## What the searchers share

The transposition table is the only structure searchers share. Each entry packs
a move, score, static evaluation, depth, generation, and bound into one 64-bit
payload, stored beside a verification word holding the mixed key exclusive-ored
with that payload. A reader recomputes the key from both words and rejects a
mismatch, so an entry caught between two writes is detected without locking. The
mixed key folds the halfmove clock class in, which keeps entries isolated across
the rule-fifty horizon exactly as the single-threaded table did. A bucket is four
such slots, which is one cache line.

Everything else is per-searcher: killers, butterfly and continuation history,
capture history, per-ply move-picker storage, principal variations, and static
evaluations. The pawn and king structure cache remains thread-local and verified
by full key, so it stays an optimization with no observable effect.

A lost race costs at most one entry. Replacement decisions come from one snapshot
per slot, so a concurrent writer can cost an entry its slot or overwrite one that
was chosen to be kept; neither outcome can produce a slot that verifies as
another position's payload.

## Main searcher and helpers

The main searcher owns everything user-visible: the clock and its soft and hard
deadlines, the reported `info` lines, the styled root that decides personality
and sacrifice questions, and the `bestmove` the search returns. Helpers exist
only to deepen the shared table, and never run the styled root.

A helper never reports progress and never decides when to stop deepening. It
observes cancellation, the hard deadline, and a shared release flag, but not the
soft-deadline decision that ends an iteration early, so the main searcher
finishing is what normally ends it. Release is deliberately distinct from an
explicit stop, which would be indistinguishable from a cancelled search. Every
helper is joined before a search returns, and each is caught individually so a
helper that fails costs its thread rather than the search.

Helpers diversify deterministically by index rather than randomly. Odd-indexed
helpers take every second depth starting one ahead, reaching deep results sooner
and leaving them in the table. Every helper rotates the root move order by its
index, so helpers disagree about which subtree to establish first. Without that
rotation, extra threads would repeat the same work rather than widening coverage.

Reported nodes and telemetry are summed across searchers. Every telemetry counter
records work performed, so a sum keeps the relationships between them true of the
whole search.

## Determinism

One thread is deterministic. It runs the main searcher inline with no scope, no
spawned thread, and a node limit measured against its own count, so a fixed-node
search is exact and repeatable. Every fixed-node fixture, aggression gate,
acceptance contract, and recorded strength series measures this configuration,
and `Threads` defaults to one so they continue to.

More than one thread is not reproducible move for move. The tree the helpers
explore depends on how their timing interleaves, so the selected move and score
may differ between runs of the same position at the same limit. This is inherent
to lazy SMP and is not a defect. With helpers running, the node limit bounds the
search as a whole rather than one searcher, and because the shared total refreshes
on the existing polling cadence it can be overshot by less than one interval per
searcher. A fixed-node comparison across different thread counts is therefore not
a like-for-like measurement, and strength must be measured at equal time.

## Measurement protocol

Parallel strength must be measured at an equal time control, never at fixed
nodes, because a node budget shared between searchers does not describe the same
amount of work per searcher.

Build one binary and run it against itself at differing thread counts, so the
only difference between the sides is the option value. The bundled
`tools/data/openings.epd` holds 48 positions, which bounds a `run_match.py` run
to 96 games; larger runs need a larger suite, as the recorded series use:

```sh
python3 tools/run_match.py \
  --engine /path/to/jakgro \
  --candidate-aggression 75 \
  --baseline-aggression 75 \
  --candidate-name Threads-8 \
  --baseline-name Threads-1 \
  --time-control 10+0.1 \
  --threads 8 \
  --games 96 \
  --pgn artifacts/lazy-smp.pgn \
  --manifest artifacts/lazy-smp.json
```

On a host without `cutechess-cli`, `selfplay` accepts the same `--threads` option
and sends it during every handshake, including after an engine restart, and is
what the recorded series use for high game counts:

```sh
./target/release/selfplay \
  --engine /path/to/jakgro \
  --candidate-aggression 75 --baseline-aggression 75 \
  --candidate-name Threads-8 --baseline-name Threads-1 \
  --games 1200 --time-control 1.0+0.01 --threads 8 \
  --openings docs/tuning/data/selective-search-confirmation.epd \
  --concurrency 8 \
  --pgn artifacts/lazy-smp.pgn --results-json artifacts/lazy-smp.summary.json
```

`--threads` currently configures both sides identically, so measuring one count
against another requires two invocations with differing values compared through
their summaries, or a per-side option once the tooling grows one.

Evaluate the result with `tools/run_sprt.py` and validate the PGN with
`tools/analyze_match.py`, exactly as the existing series do. Two conditions must
hold before any parallel Elo claim is recorded:

- the match runs at equal time control on an otherwise-idle host with enough
  physical cores for the higher thread count, since oversubscription measures the
  scheduler rather than the search; and
- concurrency is set so that total engine threads across simultaneous games do
  not exceed the host's physical cores.

Searched-node throughput can be compared separately with
`tools/measure_search_efficiency.py` at fixed time, which reports completed depth
and NPS. Throughput scaling is necessary but not sufficient: lazy SMP can raise
NPS substantially while converting little of it into strength, so a node or depth
improvement must never be reported as an Elo result.
