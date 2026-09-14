# NNUE training recipe consumed by ./autoresearch.sh (sourced by bash).
#
# The harness hashes this file together with the feature helper, the corpus
# and the trainer sources; a change here retrains, an unchanged recipe reuses
# the cached network.

# Labelled corpus: `FEN;white-outcome;white-score-cp` lines, gzip allowed.
NNUE_TRAINING_SOURCE=docs/tuning/data/evaluation-refit-pilot/training.filtered.txt.gz
NNUE_DEVELOPMENT_SOURCE=docs/tuning/data/evaluation-refit-pilot/development.filtered.txt.gz

# Extra arguments for `train_nnue.py prepare`.
NNUE_PREPARE_ARGS=(--deduplicate --drop-development-overlap)

# Extra arguments for `train_nnue.py train`.
NNUE_TRAIN_ARGS=(--epochs 10 --batch-size 256 --rate 0.001 --l2 1e-6 --seed 75 --lambda 0.0)
