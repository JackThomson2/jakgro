"""Validated, hash-bound NNUE datasets prepared by the Rust feature extractor."""

from __future__ import annotations

from collections import Counter
from dataclasses import dataclass
import gzip
import hashlib
import json
import math
from pathlib import Path
import struct
import subprocess
import tempfile

try:
    from .nnue_format import INPUTS, HIDDEN, ACTIVATION, OUTPUT_SCALE, features, require
except ImportError:
    from nnue_format import INPUTS, HIDDEN, ACTIVATION, OUTPUT_SCALE, features, require

HEADER = "# jakgro-nnue-data-v1\t12288\t128\t255\t64\n"
COLUMNS = "fen\tkey\tstm\toutcome\tteacher_cp\twhite\tblack\n"
ARCHITECTURE = {"version": 1, "features": INPUTS, "hidden": HIDDEN,
                "activation": ACTIVATION, "output_scale": OUTPUT_SCALE}
MAX_EXPANDED_BYTES = 512 * 1024 * 1024
MAX_ROWS = 2_000_000


def sha256(path: Path) -> str:
    result = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1 << 20), b""):
            result.update(block)
    return result.hexdigest()


@dataclass(frozen=True)
class Sample:
    fen: str
    key: str
    stm: int
    outcome: float
    teacher: float | None
    white: tuple[int, ...]
    black: tuple[int, ...]

    def feature_key(self) -> bytes:
        # Ignore turn and omitted metadata, and also match color/rank mirrors.
        # This is deliberately stricter than output equality for one turn.
        first, second = sorted((self.white, self.black))
        packed = struct.pack("<B", len(first)) + struct.pack(f"<{len(first)}H", *first)
        packed += struct.pack(f"<{len(second)}H", *second)
        return hashlib.sha256(packed).digest()


def read_records(path: Path, max_rows: int = MAX_ROWS) -> list[Sample]:
    samples = []
    with path.open(encoding="utf-8", newline="") as source:
        require(source.readline(256) == HEADER, f"{path}: unsupported feature schema")
        require(source.readline(256) == COLUMNS, f"{path}: wrong dataset columns")
        line_number = 2
        while True:
            line = source.readline(8193)
            if not line:
                break
            line_number += 1
            require(len(line) <= 8192 and line.endswith("\n"), f"{path}:{line_number}: invalid record length")
            fields = line[:-1].split("\t")
            require(len(fields) == 7, f"{path}:{line_number}: expected seven fields")
            fen, key, stm, outcome, teacher, white, black = fields
            try:
                turn = int(stm)
                result = float(outcome)
                score = None if teacher == "-" else float(teacher)
                white_features = features(int(value) for value in white.split(","))
                black_features = features(int(value) for value in black.split(","))
                require(turn in (0, 1) and result in (0.0, 0.5, 1.0), "invalid turn or outcome")
                require(score is None or (math.isfinite(score) and abs(score) <= 32000), "invalid teacher score")
                require(len(white_features) == len(black_features), "perspective piece counts differ")
                fen_fields, key_fields = fen.split(), key.split()
                require(len(fen_fields) == 6 and len(key_fields) == 4, "invalid FEN/key fields")
                require(fen_fields[:3] == key_fields[:3], "canonical key disagrees with FEN")
                require(key_fields[3] in (fen_fields[3], "-"), "invalid canonical en passant")
                require(fen_fields[1] == ("w" if turn == 0 else "b"), "turn disagrees with FEN")
                require(all(character not in fen + key for character in ("\r", "\0", ";")), "invalid record text")
            except ValueError as error:
                raise ValueError(f"{path}:{line_number}: {error}") from error
            samples.append(Sample(fen, key, turn, result, score, white_features, black_features))
            require(len(samples) <= max_rows, f"{path}: sample limit exceeded")
    require(bool(samples), f"{path}: empty dataset")
    return samples


def write_records(path: Path, samples: list[Sample]) -> None:
    with path.open("w", encoding="utf-8", newline="\n") as output:
        output.write(HEADER + COLUMNS)
        for sample in samples:
            teacher = "-" if sample.teacher is None else repr(sample.teacher)
            output.write("\t".join((sample.fen, sample.key, str(sample.stm), repr(sample.outcome), teacher,
                                    ",".join(map(str, sample.white)), ",".join(map(str, sample.black)))) + "\n")


def unique_records(samples: list[Sample], deduplicate: bool) -> tuple[list[Sample], dict[str, int]]:
    seen_keys, seen_features = set(), set()
    kept = []
    dropped = Counter()
    for sample in samples:
        identity = sample.feature_key()
        reason = "canonical_duplicate" if sample.key in seen_keys else "feature_duplicate" if identity in seen_features else None
        if reason is not None:
            require(deduplicate, f"{reason}: use --deduplicate to explicitly keep only the first occurrence")
            dropped[reason] += 1
            continue
        seen_keys.add(sample.key)
        seen_features.add(identity)
        kept.append(sample)
    return kept, dict(dropped)


def check_split(training: list[Sample], development: list[Sample]) -> None:
    for name, samples in (("training", training), ("development", development)):
        require(bool(samples), f"{name} split is empty")
        unique_records(samples, False)
    keys = {sample.key for sample in training}
    require(not any(sample.key in keys for sample in development), "canonical training/development overlap")
    identities = {sample.feature_key() for sample in training}
    require(not any(sample.feature_key() in identities for sample in development),
            "feature-identical training/development overlap (including omitted metadata or color mirrors)")


def stats(samples: list[Sample]) -> dict:
    support = [0] * INPUTS
    buckets = [[0] * 16 for _ in range(2)]
    for sample in samples:
        for side, indices in enumerate((sample.white, sample.black)):
            buckets[side][indices[0] // 768] += 1
            for index in indices:
                support[index] += 1
    return {"rows": len(samples), "white_to_move": sum(sample.stm == 0 for sample in samples),
            "white_outcomes": dict(sorted(Counter(str(sample.outcome) for sample in samples).items())),
            "teacher_rows": sum(sample.teacher is not None for sample in samples),
            "king_bucket_rows": buckets, "feature_support": support}


def expand(source: Path, destination: Path) -> str:
    digest = hashlib.sha256()
    opener = gzip.open if source.suffix == ".gz" else open
    count = 0
    with opener(source, "rb") as stream, destination.open("wb") as output:
        for block in iter(lambda: stream.read(1 << 20), b""):
            count += len(block)
            require(count <= MAX_EXPANDED_BYTES, "expanded dataset exceeds 512 MiB; partition the corpus")
            digest.update(block)
            output.write(block)
    return digest.hexdigest()


def drop_development_overlap(training: list[Sample], development: list[Sample]) -> tuple[list[Sample], int]:
    """Removes development rows that duplicate any training position or its mirror."""
    keys = {sample.key for sample in training}
    identities = {sample.feature_key() for sample in training}
    kept = [sample for sample in development
            if sample.key not in keys and sample.feature_key() not in identities]
    return kept, len(development) - len(kept)


def prepare(helper: Path, training: Path, development: Path, output: Path, deduplicate: bool = False,
            drop_overlap: bool = False) -> dict:
    helper, training, development = (path.resolve(strict=True) for path in (helper, training, development))
    output = output.absolute()
    require(not output.exists(), "output already exists; choose a new directory")
    require(training != development, "training and development are the same file")
    require(all(path.is_file() for path in (helper, training, development)), "inputs must be regular files")
    before = {str(path): sha256(path) for path in (helper, training, development)}
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".nnue-prepare-", dir=output.parent) as temporary:
        staging = Path(temporary)
        manifest = {"schema_version": 1, "architecture": ARCHITECTURE, "helper_sha256": before[str(helper)],
                    "deduplicate": deduplicate, "drop_development_overlap": drop_overlap, "splits": {},
                    "split_policy": "Reject canonical and feature-identical overlap, including turn changes and color/rank mirrors; not a game/family independence certificate."}
        prepared = {}
        for name, source in (("training", training), ("development", development)):
            plain = staging / (name + ".txt")
            expanded_hash = expand(source, plain)
            raw = staging / (name + ".raw.tsv")
            log = staging / (name + ".prepare.log")
            command = [str(helper), "prepare", str(plain)]
            with raw.open("wb") as destination, log.open("wb") as diagnostics:
                result = subprocess.run(command, stdout=destination, stderr=diagnostics, timeout=300)
            require(result.returncode == 0, f"{name} preparation failed: {log.read_text(errors='replace')[-3000:]}")
            original = read_records(raw)
            records, dropped = unique_records(original, deduplicate)
            if name == "development" and drop_overlap:
                records, overlapping = drop_development_overlap(prepared["training"], records)
                dropped["training_overlap"] = overlapping
                require(bool(records), "every development row overlaps the training split")
            prepared[name] = records
            target = staging / (name + ".tsv")
            write_records(target, records)
            manifest["splits"][name] = {"source": str(source), "source_sha256": before[str(source)],
                "expanded_sha256": expanded_hash, "raw_rows": len(original), "dropped": dropped,
                "prepared_file": target.name, "prepared_sha256": sha256(target), **stats(records)}
            plain.unlink()
            raw.unlink()
        check_split(prepared["training"], prepared["development"])
        require(before == {str(path): sha256(path) for path in (helper, training, development)},
                "inputs changed during preparation")
        (staging / "manifest.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
        require(not output.exists(), "output appeared during preparation")
        staging.rename(output)
    return manifest


def load_dataset(path: Path, helper: Path) -> tuple[dict, list[Sample], list[Sample]]:
    manifest = json.loads((path / "manifest.json").read_text())
    require(manifest["schema_version"] == 1 and manifest["architecture"] == ARCHITECTURE, "unsupported dataset schema")
    require(manifest["helper_sha256"] == sha256(helper), "feature/scoring helper differs from prepared dataset")
    samples = []
    for name in ("training", "development"):
        record = manifest["splits"][name]
        require(record["prepared_file"] == name + ".tsv", "unsafe prepared filename")
        source = path / record["prepared_file"]
        require(sha256(source) == record["prepared_sha256"], f"{name} dataset hash mismatch")
        rows = read_records(source)
        actual = stats(rows)
        require(actual == {key: record[key] for key in actual}, f"{name} manifest statistics mismatch")
        samples.append(rows)
    check_split(*samples)
    return manifest, samples[0], samples[1]
