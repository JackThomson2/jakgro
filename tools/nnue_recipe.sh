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
    "${NNUE_CORPUS_TRAINING_SEED_GROUPS_LIST:-1 2 3 4 5 6 7 8;9 10 11 12 13 14 15 16;17 18 19 20 21 22 23 24;25 26 27 28 29 30 31 32;33 34 35 36 37 38 39 40;41 42 43 44 45 46 47 48;49 50 51 52 53 54 55 56;57 58 59 60 61 62 63 64}"
NNUE_CORPUS_DEVELOPMENT_SEED_GROUPS=("101")
NNUE_CORPUS_NODES=50000
NNUE_CORPUS_RANDOM_PLIES=8
NNUE_CORPUS_SKIP_PLIES=8
# Aggression profiles for the two sides; unequal profiles keep the two games
# of a colour-reversed pair distinct.
NNUE_CORPUS_PROFILES=(75 0)
# Teacher networks for NNUE-taught seed groups: both generating sides load the
# network into the current engine build, so those rows are labelled by the
# neural engine rather than the handcrafted one. Each set is a previously
# published network under $ART/teachers followed by its seed groups; sets are
# separated by `|`, fields within a set by `,`, seeds within a group by spaces.
# NNUE_CORPUS_TEACHER_SETS_LIST in the environment overrides the default.
#   hce64-230c5e3023c7: trained on the 64 handcrafted-taught groups
#                       (autoresearch run 18, +62.8 Elo vs HCE-75).
#   mix80m-0b9eacd319: mirrored-feature network trained on 64 handcrafted and
#                       16 hce64-taught groups (run 21, +97.8 Elo vs HCE-75).
IFS='|' read -r -a NNUE_CORPUS_TEACHER_SETS <<< \
    "${NNUE_CORPUS_TEACHER_SETS_LIST:-hce64-230c5e3023c7,201 202 203 204 205 206 207 208,209 210 211 212 213 214 215 216,217 218 219 220 221 222 223 224,225 226 227 228 229 230 231 232}"

# nnue_corpus_select TEACHER_NET points nnue_corpus at the handcrafted corpus
# (empty argument) or at the corpus taught by that network.
nnue_corpus_select() {
    nnue_corpus_dir="$ART/corpus/g${NNUE_CORPUS_GAMES_PER_SEED}-n${NNUE_CORPUS_NODES}-r${NNUE_CORPUS_RANDOM_PLIES}-s${NNUE_CORPUS_SKIP_PLIES}-a${NNUE_CORPUS_PROFILES[0]}v${NNUE_CORPUS_PROFILES[1]}-${BASELINE_COMMIT:0:12}"
    nnue_corpus_engine=$BASELINE
    nnue_corpus_teacher=()
    if [ -n "$1" ]; then
        nnue_corpus_dir+="-t$(sha256sum "$1" | cut -c1-12)"
        nnue_corpus_engine=$ENGINE
        nnue_corpus_teacher=(--eval-file "$1")
    fi
}

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

# Labelled corpus: plain `FEN;white-outcome;white-score-cp` lines. Handcrafted
# rows first, then any teacher-taught rows; the mix is cached by its parts.
nnue_corpus_select ""
nnue_corpus training "${NNUE_CORPUS_TRAINING_SEED_GROUPS[@]}"
NNUE_TRAINING_SOURCE=$nnue_corpus_result
nnue_corpus development "${NNUE_CORPUS_DEVELOPMENT_SEED_GROUPS[@]}"
NNUE_DEVELOPMENT_SOURCE=$nnue_corpus_result
nnue_mixed_parts=("$NNUE_TRAINING_SOURCE")
for set in "${NNUE_CORPUS_TEACHER_SETS[@]}"; do
    [ -n "$set" ] || continue
    IFS=',' read -r -a fields <<< "$set"
    teacher="$ART/teachers/${fields[0]}.nnue"
    [ -f "$teacher" ] || { log "missing teacher network $teacher"; exit 1; }
    nnue_corpus_select "$teacher"
    nnue_corpus training "${fields[@]:1}"
    nnue_mixed_parts+=("$nnue_corpus_result")
done
if [ ${#nnue_mixed_parts[@]} -gt 1 ]; then
    mixed="$ART/corpus/mixed-$(cat "${nnue_mixed_parts[@]}" | sha256sum | cut -c1-16).txt"
    if [ ! -f "$mixed" ]; then
        cat "${nnue_mixed_parts[@]}" >"$mixed.tmp"
        mv "$mixed.tmp" "$mixed"
    fi
    NNUE_TRAINING_SOURCE=$mixed
fi

# Extra arguments for `nnue-data prepare`.
NNUE_PREPARE_ARGS=(--deduplicate --drop-development-overlap)

# Extra arguments for `nnue-data train`.
NNUE_TRAIN_ARGS=(--epochs 30 --batch-size 4096 --rate 0.004 --rate-decay 0.9 --l2 1e-6 --seed 75 --lambda 0.0)
