#!/usr/bin/env bash
# Autoresearch benchmark: Elo of the NNUE-enabled engine at Aggression 75
# against the frozen handcrafted-evaluation engine at Aggression 75.
#
# Workload (deterministic: one thread, fixed nodes, seeded training):
#   1. build the engine, the self-play arbiter and the tuning-only feature helper;
#   2. build (once, cached) the baseline engine from the pinned commit;
#   3. prepare the NNUE corpus and train a network from tools/nnue_recipe.sh,
#      cached by the hash of every input;
#   4. play a paired fixed-node match NNUE-75 vs baseline HCE-75 and report Elo;
#   5. play a shorter NNUE-75 vs NNUE-0 match for style retention;
#   6. sample fixed-node throughput of both evaluators.
#
# Prints `METRIC name=value` lines; exits non-zero on any failure.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

BASELINE_COMMIT=${BASELINE_COMMIT:-5c242b79e7f273e0b224b0422d6cdad388febaee}
GAMES=${GAMES:-2048}
NODES=${NODES:-50000}
STYLE_GAMES=${STYLE_GAMES:-512}
CONCURRENCY=${CONCURRENCY:-88}
OPENINGS=${OPENINGS:-docs/tuning/data/selective-search-confirmation.epd}
NPS_NODES=${NPS_NODES:-300000}
NPS_REPEATS=${NPS_REPEATS:-3}
ART=artifacts/autoresearch
mkdir -p "$ART"

log() { printf 'autoresearch: %s\n' "$*" >&2; }
sha() { cat "$@" | sha256sum | cut -c1-16; }
json() { python3 -c "import json,sys; d=json.load(open(sys.argv[1]))
for key in sys.argv[2].split('.'): d = d[int(key)] if key.lstrip('-').isdigit() else d[key]
print(d)" "$1" "$2"; }

# 1. Candidate binaries.
log "building candidate"
cargo build --release --locked --bin jakgro --bin selfplay 2>&1 | tail -1 >&2
cargo build --release --locked --features tuning --bin nnue-data 2>&1 | tail -1 >&2
ENGINE=target/release/jakgro
RUNNER=target/release/selfplay
HELPER=target/release/nnue-data

# 2. Frozen baseline engine (handcrafted evaluation) from the pinned commit.
BASELINE="$ART/baseline/jakgro-${BASELINE_COMMIT:0:12}"
if [ ! -x "$BASELINE" ]; then
    log "building baseline $BASELINE_COMMIT"
    tree=$(mktemp -d "$ART/baseline-src.XXXXXX")
    git worktree add --detach "$tree" "$BASELINE_COMMIT" >&2
    CARGO_TARGET_DIR="$PWD/$ART/baseline-target" cargo build --release --locked \
        --manifest-path "$tree/Cargo.toml" --bin jakgro 2>&1 | tail -1 >&2
    mkdir -p "$ART/baseline"
    cp "$ART/baseline-target/release/jakgro" "$BASELINE"
    git worktree remove --force "$tree"
fi

# 3. Corpus and network, cached by input hashes.
source tools/nnue_recipe.sh
DATA_KEY=$(sha "$HELPER" "$NNUE_TRAINING_SOURCE" "$NNUE_DEVELOPMENT_SOURCE" \
    tools/nnue_data.py tools/nnue_format.py <(printf '%s\n' "${NNUE_PREPARE_ARGS[@]}"))
DATA="$ART/data/$DATA_KEY"
if [ ! -f "$DATA/manifest.json" ]; then
    log "preparing corpus -> $DATA"
    rm -rf "$DATA"
    python3 tools/train_nnue.py prepare --helper "$HELPER" \
        --training "$NNUE_TRAINING_SOURCE" --development "$NNUE_DEVELOPMENT_SOURCE" \
        --output-dir "$DATA" "${NNUE_PREPARE_ARGS[@]}" >"$ART/prepare.log"
fi
NET_KEY=$(sha "$DATA/manifest.json" tools/train_nnue.py <(printf '%s\n' "${NNUE_TRAIN_ARGS[@]}"))
NET_DIR="$ART/nets/$NET_KEY"
if [ ! -f "$NET_DIR/network.nnue" ]; then
    log "training network -> $NET_DIR"
    rm -rf "$NET_DIR"
    python3 tools/train_nnue.py train --helper "$HELPER" --data-dir "$DATA" \
        --output-dir "$NET_DIR" "${NNUE_TRAIN_ARGS[@]}" >"$ART/train.log"
fi
NET="$NET_DIR/network.nnue"
log "network $(sha256sum "$NET" | cut -c1-16) epoch $(json "$NET_DIR/report.json" selected_epoch)"

# 4. Strength: NNUE-75 vs frozen HCE-75, paired fixed-node games.
log "strength match: $GAMES games at $NODES nodes"
rm -f "$ART"/strength.*
python3 tools/run_sprt.py --runner "$RUNNER" --engine "$ENGINE" --baseline-engine "$BASELINE" \
    --candidate-eval-file "$NET" --candidate-aggression 75 --baseline-aggression 75 \
    --candidate-name NNUE-75 --baseline-name HCE-75 --allow-identical-binaries \
    --games "$GAMES" --nodes "$NODES" --concurrency "$CONCURRENCY" --openings "$OPENINGS" \
    --pgn "$ART/strength.pgn" >"$ART/strength.log" 2>&1 || { tail -5 "$ART/strength.log" >&2; exit 1; }
python3 tools/analyze_match.py --pgn "$ART/strength.pgn" --json "$ART/strength.summary.json" >/dev/null
ELO=$(json "$ART/strength.sprt.json" result.elo)
[ "$ELO" != None ] || { log "score at an extreme; Elo undefined"; exit 1; }

# 5. Style: NNUE-75 vs NNUE-0 forcing-move rate, same binary and network.
log "style match: $STYLE_GAMES games"
rm -f "$ART"/style.*
python3 tools/run_sprt.py --runner "$RUNNER" --engine "$ENGINE" \
    --candidate-eval-file "$NET" --baseline-eval-file "$NET" \
    --candidate-aggression 75 --baseline-aggression 0 \
    --games "$STYLE_GAMES" --nodes "$NODES" --concurrency "$CONCURRENCY" --openings "$OPENINGS" \
    --pgn "$ART/style.pgn" >"$ART/style.log" 2>&1 || { tail -5 "$ART/style.log" >&2; exit 1; }
python3 tools/analyze_match.py --pgn "$ART/style.pgn" --json "$ART/style.summary.json" >/dev/null

# Handcrafted reference for the same style ratio, computed once per baseline.
HCE_STYLE="$ART/baseline/style-${BASELINE_COMMIT:0:12}-$STYLE_GAMES-$NODES.summary.json"
if [ ! -f "$HCE_STYLE" ]; then
    log "handcrafted style reference"
    python3 tools/run_sprt.py --runner "$RUNNER" --engine "$BASELINE" \
        --candidate-aggression 75 --baseline-aggression 0 \
        --games "$STYLE_GAMES" --nodes "$NODES" --concurrency "$CONCURRENCY" --openings "$OPENINGS" \
        --pgn "$ART/baseline/style.pgn" >"$ART/baseline/style.log" 2>&1
    python3 tools/analyze_match.py --pgn "$ART/baseline/style.pgn" --json "$HCE_STYLE" >/dev/null
fi

# 6. Throughput on the frozen performance suite.
log "throughput"
python3 tools/measure_nps.py --engine "$ENGINE" --eval-file "$NET" --nodes "$NPS_NODES" \
    --repeats "$NPS_REPEATS" >"$ART/nps-nnue.json"
python3 tools/measure_nps.py --engine "$BASELINE" --nodes "$NPS_NODES" \
    --repeats "$NPS_REPEATS" >"$ART/nps-hce.json"

python3 - "$ART" "$HCE_STYLE" <<'EOF'
import json, sys
art, hce_style = sys.argv[1], sys.argv[2]
load = lambda path: json.load(open(path))
sprt = load(f"{art}/strength.sprt.json")["result"]
strength = load(f"{art}/strength.summary.json")["style"]
style = load(f"{art}/style.summary.json")["style"]
hce = load(hce_style)["style"]
nnue_nps, hce_nps = load(f"{art}/nps-nnue.json")["median_nps"], load(f"{art}/nps-hce.json")["median_nps"]
rate = lambda block, side: block[side]["forcing_moves_per_100_moves"]
print(f"METRIC elo={sprt['elo']:.1f}")
print(f"METRIC elo_ci_low={sprt['elo_ci95'][0]:.1f}")
print(f"METRIC elo_ci_high={sprt['elo_ci95'][1]:.1f}")
print(f"METRIC score_percent={sprt['score_percent']:.2f}")
print(f"METRIC forcing_rate_ratio={rate(strength, 'candidate') / rate(strength, 'baseline'):.3f}")
print(f"METRIC style_forcing_ratio={rate(style, 'candidate') / rate(style, 'baseline'):.3f}")
print(f"METRIC hce_style_forcing_ratio={rate(hce, 'candidate') / rate(hce, 'baseline'):.3f}")
print(f"METRIC nnue_nps_ratio={nnue_nps / hce_nps:.3f}")
print(f"METRIC nnue_knps={nnue_nps / 1000:.0f}")
EOF
report="$NET_DIR/report.json"
printf 'METRIC train_selected_epoch=%s\n' "$(json "$report" selected_epoch)"
printf 'METRIC train_dev_outcome_mse=%s\n' "$(json "$report" selected_metrics.integer_development.outcome_mse)"
printf 'METRIC train_dev_label_mse=%s\n' "$(json "$report" selected_metrics.integer_development.label_mse)"
printf 'ASI network_sha256=%s\n' "$(json "$report" network_sha256)"
printf 'ASI games=%s nodes=%s\n' "$GAMES" "$NODES"
