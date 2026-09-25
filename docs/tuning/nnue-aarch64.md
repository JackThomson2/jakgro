# NNUE on AArch64: Graviton4 measurements

## Verdict

No AArch64 kernel change is kept. On a Graviton4 the network is about a third
of search time, against a fifth on the Xeon the earlier series measured, but
the NEON kernels as they stand measured faster than every variant built for
them: two and eight output accumulator chains in place of four, a NEON level
that inlines both kernels into the evaluation, and an update kernel that moves
sixty-four bytes per instruction with four-register `ld1` and `st1`. Each
variant searched byte-identical trees and cost 0.6–2.2% more cycles. The one
patch kept is a lint fix: `Level::supported` failed Clippy on AArch64.

Fixed-node searches are reproducible across the two architectures: the
performance suite at 1M nodes and Aggression 75 prints the same 159 `info`
and `bestmove` lines, without time and throughput fields, on the Xeon and on
the Graviton4.

## Host

An AWS Graviton4 (Neoverse V2): eight cores, 64 KiB of L1 data cache and
2 MiB of L2 per core, 36 MiB of L3, SVE2 at 128 bits, dot-product and i8mm
extensions, 4 KiB pages with transparent huge pages on `madvise`, otherwise
idle. Binaries were cross-built on x86-64 with Rust 1.94.1 for
`aarch64-unknown-linux-gnu`, linked with `aarch64-linux-gnu-gcc`, in the
release profile at the baseline target, from `8d04801` and the variants
below. The library tests were cross-built the same way and run natively
there: 330 pass.

## Where the time goes

Over `tests/data/search-performance.epd` at 3M nodes per position, one
thread, Aggression 75 and 64 MiB of hash, the search runs at 1.83M nodes per
second, median over positions (2.43M on the Xeon). Sampled with `perf` on a
build with line tables:

| Function | Share of cycles |
| --- | ---: |
| `kernels::apply`, the accumulator update | 17.7% |
| `SearchContext::probe_table` | 16.3% |
| `SearchContext::static_score` | 14.4% |
| `negamax` | 12.8% |
| `MovePicker::next` | 8.9% |
| quiet generation | 4.6% |

`static_score` has the whole evaluation inlined, the NEON output kernel with
it, and the output kernel's two loops, one per perspective, take 71% of its
samples: about 10% of all cycles, against 4.4–4.8% for the AVX-512 output
kernel on the Xeon. The bookkeeping is the remaining 4%. The update kernel
is the largest single cost: at 128 bits per vector it issues four times the
loads and stores AVX-512 does for the same row.

The output kernel is compute-bound. Of the backend stalls on memory, only
0.6% of those in `static_score` fall inside its loops; they are the pair
multiply-accumulates (`mul`, `smlal`, `smlal2`, three multiplies per eight
units) that bound it. Memory is not the network's problem either: the whole
run completes fewer than 40,000 table walks (one sample at a period of
20,011),
so the weights gain nothing from huge pages, and the update kernel's 34% of
the second-level refills does not show as stalls. Of the 2.01G cycles the
backend stalls on memory, out of 3.83G, 72% are in the table probe, which is
the largest memory cost on this core and lies outside the evaluator.

## Rejected directions

Each variant was checked node for node against `8d04801` on the performance
suite at 1M nodes at Aggression 0, 75 and 100 (160, 159 and 148 lines, all
identical), then measured over eight rounds of the suite at 3M nodes, both
binaries run back to back in each round on one pinned core, by user cycles
and instructions from `perf stat`. Each variant's rounds agree to within
about 0.7%.

|Variant|Cycles, median [range]|Instructions|
|---|---|---|
|Output kernel with eight accumulator chains (four vectors a step)|+2.2% [+1.7, +2.4]|-1.6%|
|Output kernel with two accumulator chains (one vector a step)|+2.1% [+1.8, +2.5]|+2.9%|
|NEON level inlining both kernels into the evaluation|+0.6% [+0.5, +0.8]|+4.3%|
|Update kernel with four-register `ld1`/`st1`, 64 bytes a step|+1.2% [+0.9, +1.6]|-7.0%|

**Accumulator chains.** The output kernel keeps two vectors a step, each
with its own low and high accumulator. Four chains are the fastest of the
three counts measured, in both directions, so the multiply-accumulate's
latency is not what bounds it on this core, which forwards accumulators
between dependent `smlal`s.

**Inlining the kernels.** On AArch64 the stack evaluates through the per-call
dispatch, which has nothing to select there, and the update kernel stays an
out-of-line call. A NEON level carrying the always-inlined bodies removed the
call, but the dispatched path is still compiled beside it, so `static_score`
grows from 4.8 KiB to 13.3 KiB, and the result is slower.

**Four-register loads and stores.** The autovectorised update moves 32 bytes
of each operand per iteration with `ldp`/`stp` and five pointer increments,
about eighteen instructions a step. The hand-written version moved 64 bytes
per `ld1` and `st1` with post-indexed addressing and executed 7% fewer
instructions, and ran slower: the update is bound by data movement, not by
issue, and the paired forms move it faster.

## Not measured

- SVE: this core's vectors are 128 bits, no wider than NEON, and Rust's SVE
  intrinsics are unstable, so an SVE kernel would need inline assembly.
- Dot-product instructions (`sdot`, `usdot`): they multiply bytes, and the
  squared activation of up to 255 times its weight does not fit them without
  a different output quantization and a retrained network.
- A binary tuned for the core, such as `-C target-cpu=neoverse-v2`, and any
  Apple silicon or Graviton3 host.

## Kept

`Level::supported`, which only the tests call, pushed into a new `Vec`. On
AArch64 the x86 block compiles away, and Clippy's `vec_init_then_push`
rejected the remaining push under `-D warnings` for `--all-targets`; the
list is now built from an iterator chain, the same on both architectures.

## Reproduction

```sh
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
  CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \
  AR_aarch64_unknown_linux_gnu=aarch64-linux-gnu-ar
cargo build --release --locked --target aarch64-unknown-linux-gnu --bin jakgro
cargo clippy --release --locked --target aarch64-unknown-linux-gnu --all-targets \
  -- -D warnings -A clippy::manual_slice_size_calculation
```

On the AArch64 host, run the performance suite's searches with
`go nodes 3000000` under `perf stat -e cycles:u,instructions:u taskset -c <cpu>`
for each binary in turn, and record `cpu_cycles`, `dtlb_walk`,
`l2d_cache_refill` and `stall_backend_mem` with `perf record` for the
profile.

## Limitations

- One Neoverse V2 host. Cores with different multiply-accumulate forwarding,
  load-pair throughput or vector width may rank the variants differently.
- Throughput only: no match was played, and none of the variants was kept.
