# NNUE training recipe consumed by ./autoresearch.sh (sourced by bash after
# BASELINE, RUNNER, HELPER, ART, OPENINGS and CONCURRENCY are set).
#
# The harness hashes the corpus files together with the feature helper and the
# trainer sources; a change here retrains, an unchanged recipe reuses the cached
# network. The corpus itself is generated once per settings key from
# deterministic fixed-node self-play of the frozen baseline engine.

NNUE_CORPUS_GAMES_PER_SEED=4096
# Seed groups: one cached corpus member per group, generated in one arbiter
# run. Semicolon-separated groups; NNUE_CORPUS_TRAINING_SEED_GROUPS_LIST in the
# environment overrides the training groups (tools/nnue_corpus.sh uses this to
# pre-generate members the recipe does not use yet).
IFS=';' read -r -a NNUE_CORPUS_TRAINING_SEED_GROUPS <<< \
    "${NNUE_CORPUS_TRAINING_SEED_GROUPS_LIST:-1 2 3 4 5 6 7 8;9 10 11 12 13 14 15 16;17 18 19 20 21 22 23 24;25 26 27 28 29 30 31 32}"
NNUE_CORPUS_DEVELOPMENT_SEED_GROUPS=("101")
NNUE_CORPUS_NODES=50000
NNUE_CORPUS_RANDOM_PLIES=8
NNUE_CORPUS_SKIP_PLIES=8
# Aggression profiles for the two sides; unequal profiles keep the two games
# of a colour-reversed pair distinct.
NNUE_CORPUS_PROFILES=(75 0)
# Optional teacher network: when set, both generating sides load it, so the
# corpus is labelled by the NNUE engine rather than the handcrafted one. The
# file must be a previously published network; record its origin here.
NNUE_CORPUS_TEACHER_NET=

nnue_corpus_dir="$ART/corpus/g${NNUE_CORPUS_GAMES_PER_SEED}-n${NNUE_CORPUS_NODES}-r${NNUE_CORPUS_RANDOM_PLIES}-s${NNUE_CORPUS_SKIP_PLIES}-a${NNUE_CORPUS_PROFILES[0]}v${NNUE_CORPUS_PROFILES[1]}-${BASELINE_COMMIT:0:12}"
nnue_corpus_engine=$BASELINE
nnue_corpus_teacher=()
if [ -n "$NNUE_CORPUS_TEACHER_NET" ]; then
    nnue_corpus_dir+="-t$(sha256sum "$NNUE_CORPUS_TEACHER_NET" | cut -c1-12)"
    nnue_corpus_engine=$ENGINE
    nnue_corpus_teacher=(--eval-file "$NNUE_CORPUS_TEACHER_NET")
fi

# nnue_corpus NAME "SEEDS"... generates one cached file per seed group, then
# concatenates them into $nnue_corpus_dir/NAME-FIRST-LAST.txt and leaves that
# path in $nnue_corpus_result.
nnue_corpus() {
    local name=$1
    shift
    local members=()
    for group in "$@"; do
        local seeds=($group)
        local member="$nnue_corpus_dir/seeds-${seeds[0]}-${seeds[-1]}.txt"
        if [ ! -f "$member" ]; then
            log "generating corpus seeds $group -> $member"
            cargo build --release --locked --features tuning --bin tune 2>&1 | tail -1 >&2
            python3 tools/generate_nnue_corpus.py --engine "$nnue_corpus_engine" --runner "$RUNNER" \
                "${nnue_corpus_teacher[@]}" \
                --tune target/release/tune --openings "$OPENINGS" --output "$member" \
                --games-per-seed "$NNUE_CORPUS_GAMES_PER_SEED" --seeds "${seeds[@]}" \
                --nodes "$NNUE_CORPUS_NODES" --random-plies "$NNUE_CORPUS_RANDOM_PLIES" \
                --skip-plies "$NNUE_CORPUS_SKIP_PLIES" \
                --candidate-aggression "${NNUE_CORPUS_PROFILES[0]}" \
                --baseline-aggression "${NNUE_CORPUS_PROFILES[1]}" \
                --concurrency "$CONCURRENCY" >&2
        fi
        members+=("$member")
    done
    local first=($1)
    local last=(${*: -1})
    nnue_corpus_result="$nnue_corpus_dir/$name-${first[0]}-${last[-1]}.txt"
    if [ ! -f "$nnue_corpus_result" ]; then
        cat "${members[@]}" >"$nnue_corpus_result.tmp"
        mv "$nnue_corpus_result.tmp" "$nnue_corpus_result"
    fi
}

# Labelled corpus: plain `FEN;white-outcome;white-score-cp` lines.
nnue_corpus training "${NNUE_CORPUS_TRAINING_SEED_GROUPS[@]}"
NNUE_TRAINING_SOURCE=$nnue_corpus_result
nnue_corpus development "${NNUE_CORPUS_DEVELOPMENT_SEED_GROUPS[@]}"
NNUE_DEVELOPMENT_SOURCE=$nnue_corpus_result

# Extra arguments for `nnue-data prepare`.
NNUE_PREPARE_ARGS=(--deduplicate --drop-development-overlap)

# Extra arguments for `nnue-data train`.
NNUE_TRAIN_ARGS=(--epochs 30 --batch-size 1024 --rate 0.002 --rate-decay 0.9 --l2 1e-6 --seed 75 --lambda 0.0)
