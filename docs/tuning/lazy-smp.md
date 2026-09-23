# Lazy SMP

Jakgro searches on a configurable number of threads. `Threads` defaults to one
and is bounded from one to 128. No Elo claim is recorded here: this document
describes the design, states what is and is not reproducible, and specifies the
protocol that a strength measurement must follow. The first eight-thread
implementation checks are recorded in [the bullet validation report](lazy-smp-bullet.md).

## What the searchers share

The transposition table is the shared mutable search-knowledge structure.
Searchers also share read-only inputs and the network, cancellation/deadlines,
and batched node accounting. Each table entry packs a move, score, static
evaluation, depth, generation, and bound into one 64-bit
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

A lost write normally costs a cached entry. A node probes its bucket on entry and
stores into it on exit, and the store decides its replacement from the words
the probe read rather than reading the bucket again, so a store issues no bucket
reloads and at most two stores. A candidate without a move inherits the recorded
move; if the resulting payload equals the snapshot, no write is needed. That
snapshot may be stale by then: the node's own subtree or another searcher may
have written the bucket since, and a stale decision can cost the entry its slot,
overwrite one a fresh read would have kept, or restore an older result of the
same position. Conversely, suppressing an unchanged snapshot can leave a newer
entry alone. The two atomic words are not a transactional pair: mixed pairs
normally fail verification, but XOR verification is probabilistic, not an
absolute guarantee against a wrong-key collision.

## Main searcher and helpers

The main searcher owns everything user-visible: the clock and its soft and hard
deadlines, the reported `info` lines, the styled root that decides personality
and sacrifice questions, and the `bestmove` the search returns. Helpers exist
only to deepen the shared table, and never run the styled root.

A helper neither reports progress nor formats UCI principal-variation strings;
it keeps the internal move variation needed for iterative deepening. It observes
cancellation, the hard deadline, a shared release flag, and its node/depth/mate
limits, but not the main searcher's soft-deadline decision, so the main searcher
finishing is what normally ends it. Release is deliberately distinct from an
explicit stop, which would be indistinguishable from a cancelled search. Every
helper is joined before a search returns. Helpers are caught individually in
unwinding builds; the release profile uses aborting panics, so a panic there
still terminates the process.

Helpers diversify deterministically by index rather than randomly. Odd-indexed
helpers take every second depth starting one ahead, reaching deep results sooner
and leaving them in the table. Helpers rotate the final ordered root alternatives
by their zero-based index plus one, after scoring and numeric tie-breaking. An
available previous-PV or hash move stays first to establish alpha cheaply; with
no preferred move, the whole order is rotated. The main searcher is not rotated.
This gives helpers different alternative orders for identical ordering inputs,
although small move sets and racing table/history inputs can still produce
coincident orders. Diversification is not itself a strength guarantee.

Reported nodes include the main searcher's current nodes and the helpers' last
published counts at each completed main iteration. The final returned `info`
remains that iteration's snapshot, not a recount after helper joins or an
abandoned iteration. Telemetry is merged from the completed worker outcomes.
Use independent `go`-to-`bestmove` timing when measuring end-to-end latency rather
than treating the last `info` timestamp as the whole search duration.

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
search as a whole rather than one searcher. Each searcher publishes its own
nodes and observes the shared total only on the existing polling cadence, so
between polls node accounting does not read a cache line another searcher
writes. Apart from the main searcher's first-iteration guarantee, the limit can
be overshot by less than two intervals per searcher:
the work the others have not yet published, and the publications this searcher
has not yet observed. A fixed-node comparison across different thread counts is
therefore not a like-for-like measurement, and strength must be measured at
equal time.

## Measurement protocol

Parallel strength must be measured at an equal time control, never at fixed
nodes, because a node budget shared between searchers does not describe the same
amount of work per searcher.

To compare implementations, build distinct old/new binaries with the same
compiler, flags, network, Hash, Aggression and thread count. The bundled
`tools/data/openings.epd` holds 48 positions, which bounds a `run_match.py` run
to 96 games; larger runs need a larger suite:

```sh
python3 tools/run_match.py \
  --engine /path/to/candidate --baseline-engine /path/to/baseline \
  --candidate-aggression 75 --baseline-aggression 75 \
  --candidate-name Candidate-8 --baseline-name Baseline-8 \
  --time-control 60+0.1 --threads 8 --hash 16 --games 96 \
  --pgn artifacts/lazy-smp.pgn --manifest artifacts/lazy-smp.json
```

Without `cutechess-cli`, `run_sprt.py` launches `selfplay`, forwards `--threads`
and records it in the manifest. The arbiter sends Threads during every
handshake, including after an engine restart. For example:

```sh
python3 tools/run_sprt.py \
  --runner ./target/release/selfplay \
  --engine /path/to/candidate --baseline-engine /path/to/baseline \
  --candidate-aggression 75 --baseline-aggression 75 \
  --candidate-name Candidate-8 --baseline-name Baseline-8 \
  --games 1200 --time-control 60+0.1 --threads 8 --hash 16 \
  --openings docs/tuning/data/selective-search-confirmation.epd \
  --concurrency 1 --pgn artifacts/lazy-smp.pgn
```

The selfplay clock syntax is seconds plus seconds: `60+0.1` is one minute plus
100 ms per move; `1+0.01` is an accelerated one-second plus 10-ms screen, not a
one-minute match. Its default time-forfeit grace is 250 ms, material at very
short controls. The archived bullet launcher sets and records a 10-ms grace.

These commands configure **both sides** with eight threads. Measuring one binary
at eight threads against itself at one thread needs a runner with per-side
Threads options; the bundled wrappers do not yet offer those. Comparing separate
8-v-8 and 1-v-1 self-match summaries does not establish 8-v-1 strength.

`run_sprt.py` runs a new match and then evaluates its paired results; it does not
stop the match as soon as a sequential boundary is crossed. It also replaces its
output PGN, so do not point it at an existing PGN to analyze that file. Validate
an already recorded PGN without replaying it using:

```sh
python3 tools/analyze_match.py \
  --pgn artifacts/lazy-smp.pgn --manifest artifacts/lazy-smp.manifest.json
```

Use the actual manifest filename (`lazy-smp.json` in the Cute Chess example).
Two conditions must hold before any parallel Elo claim is recorded:

- the match runs at equal time control on an otherwise-idle host with enough
  physical cores for the higher thread count, since oversubscription measures the
  scheduler rather than the search; and
- concurrency is set so that total engine threads across simultaneous games do
  not exceed the host's physical cores.

Searched-node throughput can be compared separately with
`tools/measure_search_efficiency.py` at fixed time, which reports completed depth
and NPS. Its `--threads` option configures both engines for the timed channel
alone: the fixed-depth and fixed-node channels keep measuring the deterministic
single-threaded search, which a parallel search cannot reproduce move for move,
while the timed channel's completed depth and node rate, reported as
`geometric_timed_nps_gain_percent`, compare the parallel search. Throughput
scaling is necessary but not sufficient: lazy SMP can raise NPS substantially
while converting little of it into strength, so a node or depth improvement must
never be reported as an Elo result.
