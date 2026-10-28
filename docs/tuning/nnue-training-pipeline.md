# Training pipeline for a larger network run

The next network is planned on 600 to 800 million rows, about twice the
from-scratch network's corpus, with a learning-rate schedule chosen for that
length and its intermediate networks screened in games. The tools stood in
the way in six places. This series removes each one, and every run the old
tools could make gives the same bytes as before:

| Before | After |
| --- | --- |
| `generate_nnue_corpus.py` played every corpus with the arbiter's adjudication defaults | the six adjudication thresholds reach the arbiter when given |
| the trainer's only schedule multiplied the rate by `--rate-decay` after each epoch | `--schedule cosine`, `--warmup-steps` and `--final-rate` set the rate of every step |
| a run kept only its selected epoch | `--save-every N` keeps every Nth epoch's networks, written as each epoch finishes |
| a training row took 144 bytes: 48.1 GB for the from-scratch network's 333.9M rows, 115 GB for 800M | 36 bytes: 12.0 GB and 28.8 GB |
| `prepare` peaked at 94.0 GiB on the from-scratch corpus's 360.0M raw training rows | it streams its inputs and keeps 128-bit digests, about 50 bytes a kept row |
| the x86 output kernels refused a hidden layer wider than 512 units at compile time | they fold their 32-bit sums into the total every 512 (AVX2) or 1024 (AVX-512) units |

No network was trained at the planned size and no game was played with these
tools. Each section below says what its change was measured on. The corpus
files named are the 20,000-node groups under `artifacts/gen3/n20000` of the
host that produced them, not in the repository.

## Corpus generation

`tools/generate_nnue_corpus.py` accepts `--resign-score`,
`--resign-move-count`, `--draw-move-number`, `--draw-move-count`,
`--draw-score` and `--max-moves` and forwards each to every `selfplay` run
only when it is given. An omitted threshold keeps the arbiter's default:
resignation at 800 cp held for four full moves, a draw from move 80 once the
score stays within 10 cp for ten moves, and a draw at move 200. Existing
invocations build the same commands as before. The manifest records all six
under `settings.adjudication`, with `null` for a threshold left to the
arbiter. A resigned game adds no rows after the resignation, so these
thresholds decide how many late positions of lost games a corpus holds; what
to set them to is a decision for the generation run, not made here. A
two-game self-play run with all six set ended in one resignation and one
fifty-move draw.

## Learning-rate schedule

`nnue-data train --schedule` chooses how the rate moves from one Adam step to
the next:

- `exponential`, the default and the previous behaviour:
  `rate * rate_decay^(epoch - 1)`, constant within an epoch;
- `cosine`: half a cosine period from `--rate` at the end of the warmup down
  to `--final-rate` (default 0) at the last step, that is
  `final + (rate - final) * (1 + cos(pi * (s - W) / (T - W))) / 2` at step
  `s` of `T` after a warmup of `W` steps;
- `--warmup-steps W`, with either schedule, scales the rate of each step
  `s < W` by `s / W`.

A decay other than 1 with the cosine schedule, a final rate without it, a
final rate outside `[0, rate]` and a warmup at least as long as the run are
rejected. The report records `schedule`, `final_rate` and `warmup_steps` only
when they are used, as it does `ema`, `factorize` and `qat`, so a run without
them reports what it did before.

The from-scratch network shows what the per-epoch decay costs. Its 80 epochs
of 40,758 steps decayed 0.0057 by 0.9 per epoch. The development label MSE of
its averaged weights fell from 0.010564 after the first epoch to 0.008766
after the fortieth, and by only 0.000049 more over the last forty, to the
selected 0.008717 at epoch 75. From the forty-first epoch the rate was below
1.5% of its start, and the eightieth ran at 1.4e-6. Which schedule and length
suit 600 to 800 million rows is the next experiment, not this one.

The exponential schedule computes each epoch's rate by the same repeated
`f32` multiplication the per-epoch update used, so its runs are
byte-identical to the previous trainer's (see [Reproducing](#reproducing)).

## Checkpoints

`--save-every N` writes the networks of every Nth epoch and of the last one
into `checkpoints/` as each epoch finishes. `epoch-N.nnue` holds the weights
`network.nnue` would hold: the moving average with `--ema`, the raw weights
otherwise. With a moving average, `raw-epoch-N.nnue` holds the raw weights.
Numbers are padded to the width of the last epoch so the files sort. Each
file is written under a temporary name and renamed, so a screen running
beside the trainer never loads a partial network, and the trainer prints each
path as it saves it.

Until the run completes the files are in the staging directory,
`<output>.staging/checkpoints`, which the completed run renames to the
output. A run that fails leaves them there, and another run with
`--save-every` refuses to start while that directory exists rather than
deleting it. The report lists every file with its epoch, weights, path,
SHA-256 and integer metrics. A network is 6,308,944 bytes, so an 80-epoch run
with a moving average that saves every fifth epoch keeps 32 files, 202 MB.

Saving changes nothing else. A four-epoch run with `--ema 0.999 --factorize
true --qat true --save-every 2` published the same network and raw network as
the same run without `--save-every`, and its `epoch-4.nnue` and
`raw-epoch-4.nnue` are byte-identical to them.

## Training rows as packed boards

A training row used to hold both perspectives' features, 2 × 32 `u16`, with
its count, bucket and label: 136 bytes, plus a `usize` in the shuffle order,
144 bytes a row. It now holds the board: the occupied squares as a `u64`, the
pieces in square order as 4-bit codes, the side to move and the label, 32
bytes, plus a `u32` in the order, 36 bytes a row. A shard rebuilds both
perspectives' features from the board each time it reads the row. It walks
the relative piece planes in order and each plane's squares in the
perspective's orientation, a byte swap for Black and a reversal of the bits
within each byte for a mirrored king, which yields the same sorted lists the
engine's `active_features` returns. Features are summed in the same order as
before, so the training trajectory does not change. Every row is checked when
it is loaded: the features rebuilt from its packed board must equal the
stored ones, and the stored ones must still equal the engine's own mapping of
the FEN.

On 9,148,730 rows, two epochs at batch 16,384, four runs alternating between
the binaries, with other jobs on the 96-vCPU host:

| Trainer | Peak RSS (`/usr/bin/time -v`) | Gradient phase per epoch | Adam phase per epoch |
| --- | ---: | --- | --- |
| previous | 2,448,488–2,450,504 kB | 4.36–4.45 s | 1.37–1.60 s |
| packed rows | 1,193,104–1,193,744 kB | 4.54–4.75 s | 1.31–1.62 s |

Rebuilding the features costs 3 to 9 percent of the gradient phase in these
runs, and Adam is unaffected. At 36 bytes a row, the from-scratch network's
333,889,174 training rows take 12.0 GB instead of 48.1 GB, and 800M rows take
28.8 GB instead of 115 GB. Beyond its rows' 36 bytes each, the first packed
run used 871,469 kB for the parameters, Adam's moments, the gradient shards,
the metric rows and the buffers the splits are read through.

## Dataset preparation

`nnue-data prepare` used to read each input whole and keep every kept row's
canonical key as a `String` and its feature identity as a `Vec<u16>` in two
hash sets until the end of the pass. On the from-scratch corpus, 360,049,827
raw training rows of which 333,889,174 were kept, it peaked at 98,546,880 kB
(94.0 GiB) and took 33 minutes 42 seconds. It now reads 16 MiB blocks of
whole lines, parses them in batches of 2^20 lines as before, and keeps the
first 16 bytes of each identity's SHA-256 in two sets of `u128`. For n
digests, the chance that two distinct identities share one is at most
n²/2¹²⁹, below 2⁻⁶⁵ for 2³¹ rows. A collision could only drop a unique row as
a duplicate, never keep a duplicate. Lines, line numbers, drop reasons,
statistics and messages are unchanged, with one exception: UTF-8 is now
validated a block at a time, so a file with an invalid byte and an earlier
fault of another kind reports the earlier fault first.

| Training input | Kept rows | Previous peak RSS | Streaming peak RSS | Wall time |
| --- | ---: | ---: | ---: | --- |
| group 78 and the first 1M lines of group 77 | 2,884,823 | 1,493,148 kB | 1,167,372 kB | 6.7 s → 3.1 s |
| groups 0 to 19, 26,865,981 lines | 26,247,783 | 7,764,996 kB | 2,310,360 kB | 58 s → 25 s |

Between the two inputs, the streaming pass grows by about 50 bytes per kept
row, against about 275 before, over a fixed cost of about 1.1 GB that is
mostly the 2^20-line parse batch. The outputs (`training.tsv`,
`development.tsv` and `CHECKSUMS`) were byte-identical to the previous
binary's on both inputs, and `manifest.json` differed only in the helper's
hash. At about 50 bytes a row, 800M kept rows would need about 41 GB. The
sets' load factor varies, though: from their layout they cost 39 to 78 bytes
a row, so preparation could need up to about 63 GB, plus a brief spike while
a table grows. These figures are extrapolated from the two inputs above, not
measured at that size.

## Output kernels past 512 units

The engine's output layer squares each clipped activation and multiplies it
by its weight in 32-bit lanes, two products to a lane per `pmaddwd`. A lane
is proven free of overflow for 64 products (64 · 127 · 255² < 2³¹). AVX2,
with one 8-lane accumulator per perspective, therefore asserted
`HIDDEN_SIZE <= 512`. AVX-512, with one 16-lane accumulator for both
perspectives, asserted `2 * HIDDEN_SIZE <= 1024`. A wider network did not
compile. Each kernel now folds its lanes into the 64-bit total every 512
(AVX2) or 1024 (AVX-512) units, and the bound is restated per block at
compile time. At 512 units, AVX-512 still shares one accumulator between the
perspectives. NEON already admitted 1024 units and the scalar path any
multiple of 64, so both are unchanged. A new test checks every kernel the
host runs against the scalar definition and the exact sum, with weights at
the limit and sums far past the clip.

At 512 units the change makes no difference. The release binary's
disassembly, with addresses normalised, differs only in the file name. `go
depth 12` gives the same nodes, scores and best moves on the 48 positions of
`tools/data/openings.epd` and the 10 of `tests/data/search-performance.epd`.
Single-threaded `go nodes 2000000` from ten openings, four interleaved
rounds, measured a median of 2,162,775 nodes per second before and 2,172,790
after.

Wider networks were measured in scratch copies with only `HIDDEN_SIZE`
changed; the engine and `nnue-data` compile with nothing else changed. Each
width's network was trained for one epoch on 192,850 rows and embedded. The
control is a 512-unit network trained the same way, since networks with
different weights search different trees. Each run was single-threaded on
one pinned core of a Xeon Platinum 8488C with AVX-512BW, with `go nodes
2000000` from ten openings and four rounds a run, and two runs were made:

| Width | Network file | Speed against the 512-unit control, two runs | Samples in NNUE code | Evaluation cycles per node, estimated |
| --- | ---: | --- | ---: | ---: |
| 512 | 6,308,944 bytes | — | 22.8% | ≈557 |
| 768 | 9,463,376 bytes | −7.7%, −12.2% | 29.0% | ≈705 |
| 1024 | 12,617,808 bytes | −15.1%, −19.5% | 34.0% | ≈925 |

The speed figures are the median of the per-round ratios. The host's load
varied between 1 and 17 during these runs, so the spread between the two
runs is the measurement's uncertainty. A 1024-unit network searches 15 to 20
percent fewer nodes per second, which its evaluation would have to win back
before it gains anything. The trainer's time per step at that width was not
measured. One existing kernel test,
`output_squares_each_clipped_activation_before_the_signed_product`, builds
its weights as `unit as i16 * 37`, which wraps past unit 885 and leaves the
admitted range. It fails at 1024 units and above because of that fixture,
not the kernels, and has to be rewritten before a wider network ships.

## Reproducing

The trainer comparisons used `nnue-data` built from `a183c44`, the commit
this series starts from, and from the series itself:

```sh
mkdir base && git archive a183c44 | tar -x -C base
(cd base && cargo build --release --locked --features tuning --bin nnue-data)
cargo build --release --locked --features tuning --bin nnue-data
G=artifacts/gen3/n20000
head -n 300000 $G/g0-s4000-4007-p8.txt > train.txt
head -n 30000 $G/g1-s4008-4015-p12.txt > dev.txt
target/release/nnue-data prepare --training train.txt --development dev.txt \
  --output-dir data --deduplicate --drop-development-overlap
for trainer in base/target/release/nnue-data target/release/nnue-data; do
  "$trainer" train --data-dir data --output-dir "run-${trainer%%/*}" \
    --epochs 4 --batch-size 8192 --rate 0.0057 --rate-decay 0.9 --lambda 0 \
    --seed 75 > "run-${trainer%%/*}.jsonl"
done
sha256sum run-*/network.nnue
diff <(sed -E 's/"seconds": \{[^}]*\}//' run-base.jsonl) \
  <(sed -E 's/"seconds": \{[^}]*\}//' run-target.jsonl)
```

The 288,881 prepared rows give `0a088ff4…` from both trainers, with
per-epoch lines that agree apart from their timings. With `--ema 0.999
--factorize true --qat true` added, both give the network `eedf123a…` and
the raw network `65f9f700…`. With `--save-every 2` added as well, the series
gives the same two, and its `checkpoints/epoch-4.nnue` and
`raw-epoch-4.nnue` match them. The 9,148,730-row comparison prepares the
concatenation of groups 0 to 6 against the first 200,000 lines of group 7 and
trains with `--epochs 2 --batch-size 16384` and the other settings above,
under `/usr/bin/time -v`; both trainers give `3f258030…`.

The preparation comparison runs both binaries' `prepare --deduplicate
--drop-development-overlap` on the training input in the table above, with
the first 500,000 lines of group 79 as development, under `/usr/bin/time -v`,
and compares `training.tsv`, `development.tsv` and `CHECKSUMS` with `cmp`.

The width measurement sets `HIDDEN_SIZE` in `src/engine/evaluation/nnue.rs`
to 768 or 1024 in a scratch copy. It prepares `head -n 200000` of group 78
against its lines 200,001 to 220,000 and trains each width with
`nnue-data train --epochs 1 --batch-size 8192 --rate 0.001 --lambda 0.25`.
The network is copied over `nets/jakgro.nnue` before `jakgro` is built. The
throughput driver, which searched each opening with `go nodes 2000000` in an
engine pinned with `taskset -c 40` and alternated the order of the networks
every round, was a scratch script and is not in the repository.

## Limitations

- No network was trained at the planned size, and no schedule, checkpoint or
  width was measured in games. The 768- and 1024-unit figures are throughput
  only.
- The cost of rebuilding features and the width figures come from a shared,
  noisy host. The preparation memory at 800M rows is extrapolated from two
  inputs.
- The x86 kernels were measured. NEON is unchanged, was not compiled for this
  series, and still refuses a hidden layer wider than 1024 units.
