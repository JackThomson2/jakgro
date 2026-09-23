# Transposition table series

## Verdict

Three patches landed, none of them a strength patch, and this series makes
**no Elo claim**. Its question was whether a table more closely specialised to
this engine's traffic had headroom left, and the answer, measured before
anything was built, is that the shape of the table does not: capacity has no
measurable effect at the depths the recorded controls reach, the table already
sits on huge pages on the measurement host, and quiescence, which makes most
of the probes, hits rarely but cuts off almost every time it does.

What landed is what those measurements left worth doing. The telemetry now
counts interior and quiescence table traffic apart, which is what made the
measurements possible. The table holds as many buckets as fit in the memory a
`Hash` setting names rather than the power of two below it, so 1000 MiB buys
1000 MiB rather than 512. And the table maps its own memory, aligned to a 2 MiB
page and requested as large pages where the kernel offers them, so that
behaviour no longer depends on which global allocator a binary happens to link,
a `Hash` change costs a system call rather than a write of the whole table, and
x86-64 macOS is asked for superpages.

Three directions were priced against the same numbers and not built: a denser
twelve-byte entry, a cache-resident first-level table for quiescence, and
skipping the store of a stand-pat cutoff. Each is recorded below with the
figures that rejected it, so a later attempt starts from evidence.

## Provenance

| Input | Value |
| --- | --- |
| Series base | `59cd7f2c9247cc4fd4def2a9b9dc57c355de830d` |
| Patches | `8ce6376` telemetry, `26d80df` sizing, `704941f` mapping |
| Build profile | `release`, `--locked`, one thread, default profile |
| Toolchain | `rustc 1.96.0` |
| Host | 96-thread x86_64 Linux, Intel Xeon Platinum 8488C, 105 MiB L3, THP policy `madvise` |
| Suite | the ten positions of `tests/data/search-performance.epd` |

Every figure below is from this host and these binaries. Throughput figures are
single runs on a machine that was also building, so they are indicative; node
counts are deterministic and exact.

## What the table is asked to do

Table traffic at depth ten over the suite, from the class-split telemetry the
first patch adds, on the series head at the default 16 MiB:

| Class | Probes | Hits | Cutoffs | Evaluations reused |
| --- | ---: | ---: | ---: | ---: |
| Interior, root included | 524,978 | 240,542 (45.8%) | 63,301 (12.1% of probes) | 166,442 (69.2% of hits) |
| Quiescence | 865,212 | 144,848 (16.7%) | 129,962 (15.0% of probes, 89.7% of hits) | 13,766 (92.5% of the 14,886 hits that did not cut off) |

Quiescence is 62.2% of the 1,391,428 nodes and of the probes. Its hit rate is
low because most quiescence positions are seen once, but a hit is almost always
a cutoff: the entry it finds is overwhelmingly a stand-pat result stored as a
lower bound at or above beta, and the few hits that do not cut off nearly all
supply the stand-pat evaluation instead. The interior search reads the table
very differently: nearly half its probes hit, most hits order a move rather
than cut, and seven in ten also supply the static evaluation, which is the
single largest saving the table makes for it.

## Capacity

Ten positions searched to depth ten from a cleared table, at four sizes:

| Hash | Nodes | Positions differing from 16 MiB | Time | NPS |
| ---: | ---: | --- | ---: | ---: |
| 4 MiB | 1,391,474 | seven, by at most 125 nodes | 577 ms | 2.41M |
| 16 MiB | 1,391,428 | — | 566 ms | 2.46M |
| 64 MiB | 1,391,427 | one, by one node | 587 ms | 2.37M |
| 256 MiB | 1,391,427 | one, by one node | 624 ms | 2.23M |

A whole depth-ten search of these positions is between 29,000 and 266,000
nodes. A 4 MiB table holds 262,144 entries and is the only size at which a
collision is ever forced, and it costs a hundred nodes. Sixty-one searches to
depth nine along one fixed 60-ply game, with the table kept warm between them,
show the same absence of a signal in the other direction: 5,862,939 nodes at
4 MiB, 5,513,675 at 16, 6,224,740 at 64 and 5,999,960 at 256. Larger tables
searched more, not fewer, and not monotonically; that is the noise of a
different collision pattern changing move order, not a capacity effect.

At the recorded controls, 50 ms per move and `1.0+0.01`, a move is one to a few
hundred thousand nodes, and the default table holds a million entries. Capacity
is not what a `Hash` setting buys at those controls. What it costs is visible
in the last column: a table larger than the cache is slower to probe.

## Latency and pages

`perf stat` over a twenty-million-node search of `open-king-gambit`, per node:

| Hash | NPS | IPC | LLC loads | LLC load misses | dTLB loads | dTLB load misses |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 16 MiB | 2.10M | 2.41 | 0.654 | 0.026 | 1,538 | 0.0247 |
| 256 MiB | 1.82M | 1.99 | 0.781 | 0.039 | 1,622 | 0.0326 |

At 16 MiB the table lives in this host's 105 MiB last-level cache, and a probe
misses it on one node in forty. A translation miss on one node in forty means
the table was already on 2 MiB pages: on 4 KiB pages a 256 MiB table would miss
the translation cache on nearly every probe. Reading the process's mappings
confirmed it. The base binary showed 274 MB of `AnonHugePages` at 256 MiB and
1,079 MB at 1024 MiB, because mimalloc advises its large allocations that way
and the policy here is `madvise`. Setting mimalloc's own large-page option made
no further difference (2.15M and 1.83M NPS). The trees these two searches
walked differ, so the NPS ratio between the rows is indicative rather than a
measurement of probe latency alone.

Two conclusions follow. Huge pages were not a gain to be had on this host's
shipped binary, and the third patch does not claim one; it claims that the
behaviour no longer depends on the allocator, which a test or bench binary on
the system allocator did not share. And the cost of a large table is cache,
not translation: prefetching the child's bucket before the recursive call,
which the search already does, is the tool for that, and a first-level table
would have to beat it.

## Per patch

**Class-split telemetry.** Every probe names the class of node making it, and
`SearchTelemetry` keeps interior and quiescence probes, hits, cutoffs and
recovered static evaluations apart, deriving the old totals from them so the
existing accessors, regression assertions and bench columns keep their meaning.
No search behaviour changes.

**Sizing to the request.** A bucket is chosen by the high word of the key
multiplied by the bucket count, which lies in the bucket range for any count.
Keys that share a bucket now agree in their high bits rather than their low
ones; the verification word tells them apart as before. At 16 MiB every one of
the ten depth-ten searches reports the same node count and best move as the
base: a collision is what would differ, and none is forced at that size. The
tests that built colliding keys as multiples of the bucket count, which only
the mask made collide, build them through one helper that asserts the collision.

**The table's own mapping.** The bucket array is an anonymous mapping the
table owns, carved to a 2 MiB boundary and rounded up to whole 2 MiB pages,
advised `MADV_HUGEPAGE` on Linux and requested as 2 MiB superpages on x86-64
macOS through the mapping request itself. Every request is best effort; a
refusal leaves an ordinary mapping, and a platform without mappings uses the
global allocator. The kernel rejects superpages for a process on Apple silicon,
whose 16 KiB base pages are what a process gets, so that path is compiled only
where it can succeed. Zero bytes are what an empty slot stores, so a mapping
needs no initialisation pass. Measured against the sizing patch: node counts
and best moves identical on every position at depth ten; a `Hash` change to
1024 MiB from 148.7 ms to below the timer's resolution, with 19 MB resident
until a search touches the table against 1,088 MB before; 1,067 MB of the
table reported as `AnonHugePages` after a three-million-node search, against
1,071 MB for the mimalloc-backed table. A Linux test asserts the mapping's
alignment and its huge-page advice flag. `libc` is a Unix-only dependency.

## Not built

**A denser entry.** An entry is sixteen bytes: a payload word and a
verification word holding the full mixed key. The verification is far stronger
than it needs to be, and the question was whether halving or shrinking the
entry pays. It cannot be halved: a check of sixteen bits, a move of fifteen, a
score and an evaluation of sixteen each, seven bits of depth, four of
generation and two of bound are seventy-six bits, and the evaluation cannot go,
because 69% of interior hits and 92% of the quiescence hits that do not cut off
read it. Twelve bytes, five entries to a line with a 32-bit check, is the most
the layout allows, a quarter more capacity. The capacity tables above give that
quarter nothing to buy at the recorded controls, and the last compaction of
this table, twenty-four bytes to sixteen in the second strength series, was
measured 1.4% slower with identical trees. It was not built.

**A first-level table for quiescence.** A small per-searcher table sized to
stay in cache, probed before the shared one, can serve at most the quiescence
probes that would have hit anyway: 16.7% of them. The other 83% still go to the
shared table, so the saving is bounded by a sixth of the probes times the
difference between a cache-resident and an L3 probe, against a probe and a
store added to every quiescence node. On this host that bound is about one
per cent of node time. It would also cost information: interior nodes read the
hash moves and evaluations quiescence stores, and quiescence hits the entries
interior nodes store, and neither crosses a split. It was not built.

**Skipping the store of a stand-pat cutoff.** The rationale was write traffic.
The measured cost of a store is nothing worth having: the probe has already
brought the line into the first-level cache, and last-level misses are one in
forty nodes. The entries such a store writes are the ones quiescence finds:
nine in ten quiescence hits cut off, on a lower bound at or above beta that a
stand-pat cutoff stored, and the rest reuse its evaluation. Skipping the store
would remove the table's whole contribution to quiescence. It was not built.

## Method

Everything above is reproducible through UCI on one thread. For the capacity
sweep, for each size and each position of the suite: `setoption name Hash
value N`, `setoption name Clear Hash`, `position fen …`, `go depth 10`, and
read `nodes`, `time` and `bestmove`; the game sweep plays one fixed move list
with `position startpos moves …` and `go depth 9` at every ply without
clearing. For the counters, run the engine under `perf stat -e
cycles,instructions,LLC-loads,LLC-load-misses,dTLB-loads,dTLB-load-misses`
with `go nodes 20000000` and divide by the reported nodes. For the pages,
read `AnonHugePages` from `/proc/PID/smaps_rollup` while the engine holds the
table. The class-split counters are the `interior_*` and `quiescence_*`
accessors of `SearchTelemetry`, which the bench also prints as its last eight
columns.

## Limitations

- One host, one toolchain, single runs for anything timed. The host's 105 MiB
  last-level cache holds the default table whole; a machine with a smaller
  cache pays for a 16 MiB table what this one pays for 256.
- No match was run. The three patches change no search decision at the default
  size, which is a stronger guarantee than a match for what they claim, and
  they claim no strength.
- The macOS path is compiled for x86-64 only and was not exercised: the
  measurement host is Linux. The superpage request and its fallback follow the
  documented interface, and the kernel's refusal on Apple silicon is a property
  of the kernel, not something this series measured.
- `cargo bench --bench search` panics on the base commit as on the head, in its
  null-pruning tolerance check on `win-hanging-queen`, which searches 5,285
  nodes with null pruning against 2,714 without it at depth five. The bench is
  not run by `cargo test`, and the class-split counters were read through a
  throwaway harness instead. This series does not change it.
