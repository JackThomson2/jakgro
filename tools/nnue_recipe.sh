# NNUE training recipe consumed by ./autoresearch.sh (sourced by bash after
# BASELINE, RUNNER, HELPER, ART, OPENINGS and CONCURRENCY are set).
#
# The harness hashes the corpus files together with the feature helper and the
# trainer sources; a change here retrains, an unchanged recipe reuses the cached
# network. The corpus itself is generated once per settings key from
# deterministic fixed-node self-play of the frozen baseline engine.

NNUE_CORPUS_GAMES_PER_SEED=4096
NNUE_CORPUS_TRAINING_SEEDS=(1 2 3 4 5 6 7 8)
NNUE_CORPUS_DEVELOPMENT_SEEDS=(101)
NNUE_CORPUS_NODES=50000
NNUE_CORPUS_RANDOM_PLIES=8
NNUE_CORPUS_SKIP_PLIES=8
# Aggression profiles for the two sides; unequal profiles keep the two games
# of a colour-reversed pair distinct.
NNUE_CORPUS_PROFILES=(75 0)

nnue_corpus_dir="$ART/corpus/g${NNUE_CORPUS_GAMES_PER_SEED}-n${NNUE_CORPUS_NODES}-r${NNUE_CORPUS_RANDOM_PLIES}-s${NNUE_CORPUS_SKIP_PLIES}-a${NNUE_CORPUS_PROFILES[0]}v${NNUE_CORPUS_PROFILES[1]}-${BASELINE_COMMIT:0:12}"

# nnue_corpus NAME SEED... generates $nnue_corpus_dir/NAME.txt.gz once.
nnue_corpus() {
    local name=$1
    shift
    local target="$nnue_corpus_dir/$name.txt.gz"
    if [ ! -f "$target" ]; then
        log "generating $name corpus (seeds $*) -> $target"
        cargo build --release --locked --features tuning --bin tune 2>&1 | tail -1 >&2
        python3 tools/generate_nnue_corpus.py --engine "$BASELINE" --runner "$RUNNER" \
            --tune target/release/tune --openings "$OPENINGS" --output "$target" \
            --games-per-seed "$NNUE_CORPUS_GAMES_PER_SEED" --seeds "$@" \
            --nodes "$NNUE_CORPUS_NODES" --random-plies "$NNUE_CORPUS_RANDOM_PLIES" \
            --skip-plies "$NNUE_CORPUS_SKIP_PLIES" \
            --candidate-aggression "${NNUE_CORPUS_PROFILES[0]}" \
            --baseline-aggression "${NNUE_CORPUS_PROFILES[1]}" \
            --concurrency "$CONCURRENCY" >&2
    fi
}
nnue_corpus training "${NNUE_CORPUS_TRAINING_SEEDS[@]}"
nnue_corpus development "${NNUE_CORPUS_DEVELOPMENT_SEEDS[@]}"

# Labelled corpus: `FEN;white-outcome;white-score-cp` lines, gzip allowed.
NNUE_TRAINING_SOURCE=$nnue_corpus_dir/training.txt.gz
NNUE_DEVELOPMENT_SOURCE=$nnue_corpus_dir/development.txt.gz

# Extra arguments for `train_nnue.py prepare`.
NNUE_PREPARE_ARGS=(--deduplicate --drop-development-overlap)

# Extra arguments for `train_nnue.py train`.
NNUE_TRAIN_ARGS=(--epochs 30 --batch-size 256 --rate 0.001 --l2 1e-6 --seed 75 --lambda 0.0)
