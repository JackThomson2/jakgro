#!/usr/bin/env bash
# Generates any missing NNUE corpus members declared in tools/nnue_recipe.sh
# using the same variables autoresearch.sh sets, plus any extra seed groups
# named in NNUE_CORPUS_TRAINING_SEED_GROUPS_LIST. Long-running; run under a
# supervisor rather than a timed shell.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
export BASELINE_COMMIT=${BASELINE_COMMIT:-5c242b79e7f273e0b224b0422d6cdad388febaee}
export ART=artifacts/autoresearch
export OPENINGS=${OPENINGS:-docs/tuning/data/selective-search-confirmation.epd}
export CONCURRENCY=${CONCURRENCY:-88}
export BASELINE="$ART/baseline/jakgro-${BASELINE_COMMIT:0:12}"
export ENGINE=target/release/jakgro
export RUNNER=target/release/selfplay
export HELPER=target/release/nnue-data
log() { printf 'autoresearch: %s\n' "$*" >&2; }
export NNUE_CORPUS_TRAINING_SEED_GROUPS_LIST=${NNUE_CORPUS_TRAINING_SEED_GROUPS_LIST:-"1 2 3 4 5 6 7 8;9 10 11 12 13 14 15 16;17 18 19 20 21 22 23 24;25 26 27 28 29 30 31 32"}
source tools/nnue_recipe.sh
wc -l "$NNUE_TRAINING_SOURCE" "$NNUE_DEVELOPMENT_SOURCE"
