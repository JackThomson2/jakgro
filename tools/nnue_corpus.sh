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
export NNUE_CORPUS_TRAINING_SEED_GROUPS_LIST=${NNUE_CORPUS_TRAINING_SEED_GROUPS_LIST:-"1 2 3 4 5 6 7 8;9 10 11 12 13 14 15 16;17 18 19 20 21 22 23 24;25 26 27 28 29 30 31 32;33 34 35 36 37 38 39 40;41 42 43 44 45 46 47 48;49 50 51 52 53 54 55 56;57 58 59 60 61 62 63 64"}
export NNUE_CORPUS_TEACHER_SETS_LIST=${NNUE_CORPUS_TEACHER_SETS_LIST:-"hce64-230c5e3023c7,201 202 203 204 205 206 207 208,209 210 211 212 213 214 215 216,217 218 219 220 221 222 223 224,225 226 227 228 229 230 231 232|mix80m-0b9eacd319,233 234 235 236 237 238 239 240,241 242 243 244 245 246 247 248|mix96-7d8c8fd0f3,249 250 251 252 253 254 255 256,257 258 259 260 261 262 263 264|mix128-45549474bb92@100000,265 266 267 268 269 270 271 272,273 274 275 276 277 278 279 280|mix144-54a7826df077,281 282 283 284 285 286 287 288,289 290 291 292 293 294 295 296|ob144-e3c615026b6f@100000,297 298 299 300 301 302 303 304,305 306 307 308 309 310 311 312|ob176-f1a0836dd3b6@100000,313 314 315 316 317 318 319 320,321 322 323 324 325 326 327 328"}
source tools/nnue_recipe.sh
wc -l "$NNUE_TRAINING_SOURCE" "$NNUE_DEVELOPMENT_SOURCE"
