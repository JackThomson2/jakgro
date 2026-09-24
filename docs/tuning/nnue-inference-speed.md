# NNUE inference speed series

## Verdict

Three patches landed out of five built. The evaluation now runs compiled
for the host's vector width, which executes 5.4% fewer user instructions and
1.8% fewer user cycles over the same fixed-node searches with byte-identical
trees. Interior nodes that return before searching a move keep the evaluation
they computed, which cuts evaluations per node by 5.3% and node cost by 0.5%,
and measures neutral against its parent: -0.8 Elo [-6.8, +5.1] at 50 ms per
move and -0.1 Elo [-6.0, +5.8] at `1.0+0.01`, 4096 games each. A search
telemetry counter for computed evaluations made that measurable. A VNNI
output kernel and weight-row prefetching at make-move time were built,
measured and rejected.

No Elo gain is claimed for the series. The width-specific evaluation leaves
every tree unchanged, so what it is worth follows from its speed: by the
`Elo ≈ 115 × log₂(speedup)` slope measured in
[`strength-series-two.md`](strength-series-two.md), a 1.8% cycle saving is
about three Elo at 50 ms per move, which no match here was run to confirm.

## Why this series

A profile of the base (`870d503`) put a ceiling on what the network's speed
can be worth. Built with line tables and sampled with `perf` on a Xeon
Platinum 8488C, over `tests/data/search-performance.epd` at 3M nodes per
position, one thread, Aggression 75 and 64 MiB of hash, the network took about
20% of search time:

| Function | Share of cycles |
| --- | ---: |
| `kernels::avx512::apply`, the accumulator update | 6.3–6.8% |
| `kernels::avx512::output` | 4.4–4.8% |
| `SearchContext::static_score`, with the placement diffs and dispatch inlined | 4.4–5.8% |
| `nnue::Delta::new` | 3.8–3.9% |

An evaluator that cost nothing would therefore make the search at most 1.25
times as fast. About half the network's time was bookkeeping around the two
kernels rather than the kernels themselves: the shipped binary keeps the
baseline x86-64 target, so the placement diffs and feature lists counted and
cleared bits without `popcnt`, `tzcnt` or `blsr`, and every evaluation checked
the CPU's features three times to reach kernels it could not inline. The
hottest line of `static_score` was `count_ones`.

Memory was not the constraint. Page walks were 0.65% of all cycles, so huge
pages for the 6 MiB of weights would not help, and although the update kernel
issued 53% of the loads that missed the second-level cache (the output kernel
17%), most weight rows were served from it. That agrees with the earlier
finding in [`nnue-aggression75.md`](nnue-aggression75.md) that aliasing every
row into the first-level cache is worth only 2–4%.

The rest of the node is a larger pool: `MovePicker::next` 11%, quiet
generation 8–9%, the table probe 9–13%, `move_gives_check` 4.3%, sorting about
4% and exchange evaluation 3%.

## Per patch

|Patch|Trees|Measured|
|---|---|---|
|Count the static evaluations a search computes|unchanged|telemetry only|
|Compile each evaluation for the host's vector width|byte-identical|5.4% fewer instructions, 1.8% fewer cycles|
|Keep the evaluations of nodes that return early|changed|5.3% fewer evaluations and 0.5% fewer cycles per node; neutral by match|

**Counting evaluations.** `SearchTelemetry::static_evaluations` counts every
request the search makes of the evaluator, network or handcrafted, beside the
existing counts of evaluations recovered from the table, and the search bench
prints it as its last column.

**Compiling for the host's width.** The accumulator stack a search uses now
selects a kernel level when it is built: AVX-512BW or AVX2, each with
`popcnt`, `bmi1`, `bmi2` and `lzcnt`, or the per-call dispatch otherwise. Its
evaluation runs inside one `#[target_feature]` function per level, and
everything below it is generic over a kernel token and always inlined, so the
bookkeeping is compiled for the level and both kernels are inline. A token
exists only where its detection succeeded, which is what makes the kernel
calls safe. The first version relied on `#[inline]` for the kernels, and LLVM
left them as calls: that version measured +0.15% in paired throughput. With
the kernel bodies always inlined, the AVX-512 evaluation contains no calls but
the rare refresh and panic paths.

The ten searches of the performance suite at 1M nodes print byte-identical
`info` lines, without time and throughput fields, and best moves before and
after at Aggression 0, 75 and 100 (160, 159 and 148 lines). Six alternating
rounds of the suite at 3M nodes, the engine pinned to one core, measured
89.847G against 94.958G user instructions and 41.781G against 42.565G user
cycles, medians. The host was shared, and cycles spread by up to 15% between
rounds, so the cycle figure is the less certain of the two. Wall-clock
throughput over eight rounds on a quiet core measured +0.4%, inside its own
±3% round-to-round spread. The stack test walks search-shaped trees at every
level the host supports against the independent reference.

**Keeping early evaluations.** An interior node writes the table only after
its move loop, so a node that returned through reverse futility or a null-move
cutoff discarded the evaluation it had computed, and razoring's quiescence
search at the same ply computed it again. When the probe found nothing for
the position, those exits now store the entry a quiescence stand-pat cutoff
writes: a depth-zero lower bound at the evaluation, carrying the evaluation.
The bound holds for quiescence, which may stand pat outside check; interior
nodes read a depth-zero entry only for its evaluation, since no table cutoff
or singular test accepts that depth. Razoring passes its evaluation to
quiescence.

Over the search bench's seven fixtures (72,656 nodes), the search computes
0.875 evaluations per node against 0.924, and recovers 0.287 per node from the
table against 0.250. Eight paired rounds of the performance suite at 3M nodes
measured 0.5% fewer cycles per node (range -4.6% to +0.2%, seven rounds of
eight lower) and 1.0% fewer instructions. Against its parent at Aggression 75:

|Control|Games|W-D-L|Elo|LLR (0, 5)|
|---|---:|---|---|---:|
|50 ms per move|4096|1600-886-1610|-0.8 [-6.8, +5.1]|-1.81|
|`1.0+0.01`|4096|1582-931-1583|-0.1 [-6.0, +5.8]|-1.41|

Neither sequential test reached a verdict. The saving it measures is worth
about one Elo by the slope above, below what 8192 games resolve. The style,
sacrifice-gate, acceptance and standard-acceptance fixtures and the
acceptance contract pass unchanged, and no test fixture had to be re-pinned.

## Rejected directions

**A VNNI output kernel.** `vpdpwssd` performs the pair multiply-add and the
addition into the lanes as one instruction, with the same exact sums, and the
host has both AVX-512 VNNI and AVX-VNNI. With one accumulator the compiler
emitted a single rolled loop chained on that instruction's latency, since it
cannot split an intrinsic's chain the way it splits additions. With four
chains, LLVM's machine combiner turned 30 of the 32 steps back into
`vpmaddwd` and an addition under the default tuning, and the result measured
+2.1% cycles (spread 7–11%) and +0.12% instructions with identical trees. The
four extra specialised evaluations it needed are not kept.

**Prefetching weight rows at make-move time.** Beside the child's table
prefetch, the search prefetched the rows its move removes and adds, and the
captured piece's row, under the child's king views. Prefetching all sixteen
lines of each row measured +3.3% cycles, with rounds within 1% of each other,
and +6.3% instructions; prefetching only each row's first line measured +1.3%
cycles, median of eight paired rounds (range -3.6% to +3.1%), and +3.5%
instructions. The issue cost exceeds the latency it hides, as prefetching
inside the evaluation did before at -1%.

## Not in this series

A whole-binary `-C target-cpu=x86-64-v3` build measured +2.3% fixed-node
throughput over three alternating rounds on this host, and `-C target-cpu=native`
(AVX-512 throughout) -3%. The build target is unchanged; the
[PGO build](pgo-root-verification.md) remains the measured throughput option.

## Method

Throughput was measured on fixed-node searches, so both binaries did the same
work: user cycles and instructions from `perf stat` over the performance suite
at 3M nodes per position, binaries alternating each round, the engine pinned
to CPU 40 of a 48-core, 96-thread Xeon Platinum 8488C whose sibling thread was
idle. Matches used the in-repo `selfplay` arbiter at concurrency 80 over the
2048-position `selective-search-confirmation.epd` book, each position played
once in each colour, 16 MiB of hash and one thread. Builds and tests used Rust
1.93.1; lints used Clippy 1.94.1 with `manual_slice_size_calculation`
allowed, since the base already fails it in
`bucket_allocation_uses_the_requested_size`.

## Reproduction

```sh
cargo build --release --locked --bin jakgro --bin selfplay
cargo bench --locked --bench search      # static_evaluations is the last column
python3 tools/run_sprt.py --runner target/release/selfplay \
  --engine target/release/jakgro --baseline-engine <parent> \
  --candidate-aggression 75 --baseline-aggression 75 \
  --games 4096 --movetime-ms 50 --concurrency 80 --elo0 0 --elo1 5 \
  --openings docs/tuning/data/selective-search-confirmation.epd \
  --pgn artifacts/speed/strength.pgn
```

Replace `--movetime-ms 50` with `--time-control 1.0+0.01` for the clocked
channel. For the cycle counts, run the performance suite's searches with
`go nodes 3000000` under `perf stat -e cycles:u,instructions:u taskset -c <cpu>`
for each binary in turn.

## Limitations

- One host, and only its AVX-512 VNNI path was timed. The AVX2 levels are
  exercised for correctness by the stack test but not measured for speed, and
  AArch64 keeps the per-call dispatch unchanged.
- The host was shared during the measurements; the cycle figures carry the
  spreads given beside them.
- The two matches measure the early-evaluation patch only; the
  width-specific evaluation's worth is inferred from its speed, not matched.
