# Opt-in profile-guided builds

`tools/build_pgo.py` builds an optional profile-guided-optimization (PGO) engine.
Ordinary `cargo build --release --locked` behavior is unchanged. This is a build
and validation workflow, not by itself an Elo or speed claim.

## Requirements and usage

Use the LLVM tools shipped with the selected Rust toolchain:

```sh
rustup component add llvm-tools-preview
python3 tools/build_pgo.py --output-dir artifacts/pgo-run-1
```

The output directory must not exist. Prefer a directory under ignored
`artifacts/`, or a dedicated scratch location outside the source tree. The script
never cleans or reuses an earlier run and never overwrites `target/release/jakgro`.
It does not install tools or change Cargo or Git configuration.

If LLVM tools are supplied separately, pass
`--llvm-profdata /path/to/matching/llvm-profdata`. Its LLVM major version must match
`rustc --version --verbose`; using the exact Rust component is recommended because
profile-format compatibility is not guaranteed merely by a major-version match.

The build targets the installed Rust host triple explicitly, preventing the
instrumentation flags from applying to host-side build scripts when Cargo
separates host and target units. Default CPU settings remain portable within that
triple. `--target-cpu native` is optional and produces a CPU-specific binary that
must not be distributed to incompatible machines. Cross-compilation is not
supported by this helper because it executes the resulting engine locally.

## What the script does

1. Build an uninstrumented baseline in `build/baseline`.
2. Build an instrumented engine in `build/instrumented` with an absolute
   `-Cprofile-generate` directory.
3. Train over the 48 curated opening seeds and eight supplemental tactical/endgame
   positions, at 250,000 nodes per position for each of Aggression 0, 75 and 100.
   Each search starts a new game; all searches use one thread and 16 MiB hash.
4. Wait for the engine to exit cleanly, then merge its fresh raw profiles with the
   matching `llvm-profdata`. Missing/empty profiles or failed commands abort.
5. Build a new engine in `build/optimized` with `-Cprofile-use`.
6. Compare baseline and optimized searches on the separate ten-position
   performance suite at 100,000 nodes and all selected profiles. Every completed
   iteration's score, depth, nodes and PV must match, along with best move and
   personality counters. Timing and NPS are deliberately not equality criteria.
7. Verify source and suite hashes did not change, then publish `jakgro-baseline`
   and `jakgro-pgo` alongside the profiles, logs and manifest.

The default training and validation starting records are disjoint (ignoring FEN
move counters). This is not a claim of unrelated chess structures, effective-EP
canonical separation, or independence from historical engine tuning. The engine
validates FEN legality through UCI; terminal positions with no measured best move
are rejected. Fixed-node equality on a finite suite is useful evidence, not a
proof of all-position equivalence or of stronger timed play.

Override the training input with repeated `--training-suite FILE` arguments.
Four-field EPDs with `id "name";` and six-field FEN fixtures with `; id name ;`
are accepted. Duplicate identifiers or four-field starting records are refused.
`--validation-suite`, `--nodes`, `--validation-nodes`, `--profiles`, `--hash` and
per-response/build timeouts are configurable. `--revision` supplies an optional
human-readable revision label; file hashes, rather than that label, bind the run.

## Provenance and failure handling

`manifest.json` records tool versions, tool hash, source/suite hashes, profiles,
node budgets, CPU selection, build commands/flags, binary hashes and the merged
profile hash. `training.json` and `validation.json` retain measured observations;
all compiler output is saved in stage-specific logs. Source files are hashed again
before publication. Run this on a quiescent source tree and keep the whole run
when comparing builds.

The helper rejects ambient Rust compiler flags and wrapper overrides that could
silently invalidate a controlled comparison. Release-profile environment
overrides are recorded; Cargo's normal configuration, dependency cache and build
environment still apply. Reproducing byte-identical binaries across different
paths, compilers, C toolchains or machines is not guaranteed. In particular, the
LLVM profile contains counts from the training run, not a portable model of
arbitrary future workloads.

A failed run retains its logs and a `status: failed` manifest once output creation
has begun. No validated executable is published on failed training, comparison or
input verification. Use a new output directory for every retry. Builds and
validation execute sequentially so they never race over shared stage outputs.

After a successful build, measure throughput and play paired matches at equal
time controls against the recorded baseline before deciding to deploy PGO.
Keep Aggression 75 on both sides when testing default-profile strength; retain
safety checks and forcing-play measurements. A larger NPS number alone is not an
Elo improvement.
