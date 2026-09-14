#!/usr/bin/env python3
"""Prepare, train and verify external experimental Jakgro NNUE networks on CPU.

Preparation and training run inside the tuning-only Rust helper (`nnue-data`);
this tool binds their inputs and outputs by hash, checks the exported network
against a dependency-free integer oracle, and writes a provenance report. It
never changes engine configuration or installs a model. Each output is a new,
completed directory.
"""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
import platform
import subprocess
import sys
import tempfile

try:
    from . import nnue_data as data
    from . import nnue_format as fmt
except ImportError:
    import nnue_data as data
    import nnue_format as fmt

DEFAULT_K = 0.8806824810924139


def tool_source_hashes() -> dict[str, str]:
    paths = (Path(__file__), Path(data.__file__), Path(fmt.__file__))
    return {path.name: data.sha256(path) for path in paths}


LOADED_TOOL_HASHES = tool_source_hashes()


def parity_sample(samples: list[data.Sample], maximum: int = 128) -> list[data.Sample]:
    if len(samples) <= maximum:
        return samples
    return [samples[index * (len(samples) - 1) // (maximum - 1)] for index in range(maximum)]


def verify_rust(helper: Path, model_path: Path, samples: list[data.Sample], scratch: Path) -> dict:
    fmt.require(bool(samples), "parity verification needs positions")
    before = {str(path): data.sha256(path) for path in (helper, model_path)}
    blob = model_path.read_bytes()
    network = fmt.Network.from_bytes(blob)
    fen_path, output_path = scratch / "parity.fen", scratch / "parity.tsv"
    fen_path.write_text("".join(sample.fen + "\n" for sample in samples), encoding="utf-8")
    with output_path.open("wb") as output:
        result = subprocess.run([str(helper), "score", str(model_path), str(fen_path)],
                                stdout=output, stderr=subprocess.PIPE, timeout=300)
    fmt.require(result.returncode == 0, "Rust network verification failed: " + result.stderr.decode(errors="replace")[-2000:])
    records = []
    with output_path.open() as output:
        for index, sample in enumerate(samples):
            line = output.readline(16385)
            fmt.require(0 < len(line) <= 16384 and line.endswith("\n"), f"missing or invalid Rust parity row {index}")
            fields = line.strip().split("\t")
            fmt.require(len(fields) == 3, f"invalid Rust parity fields {index}")
            actual = {"cp": int(fields[0]), "white": list(map(int, fields[1].split(","))),
                      "black": list(map(int, fields[2].split(",")))}
            expected = network.infer(sample.white, sample.black, sample.stm)
            fmt.require(all(actual[key] == expected[key] for key in actual), f"Rust/Python integer mismatch at row {index}")
            records.append({"fen": sample.fen, **expected})
        fmt.require(not output.read(1), "extra Rust parity rows")
    fmt.require(before == {str(path): data.sha256(path) for path in (helper, model_path)}, "parity inputs changed")
    return {"passed": True, "positions": len(samples), "helper_sha256": before[str(helper)],
            "network_sha256": before[str(model_path)], "exact": "unclipped accumulators and signed centipawn output",
            "records": records}


def validate_options(epochs: int, batch_size: int, rate: float, l2: float, seed: int, label_mix: float, k: float,
                     threads: int | None = None) -> None:
    fmt.require(type(epochs) is int and 1 <= epochs <= 10000, "epochs must be 1..10000; initialization is not a trained model")
    fmt.require(type(batch_size) is int and 1 <= batch_size <= 65536, "batch size must be 1..65536")
    fmt.require(math.isfinite(rate) and 0 < rate <= 1, "rate must be finite in (0,1]")
    fmt.require(math.isfinite(l2) and 0 <= l2 <= 1, "L2 must be finite in [0,1]")
    fmt.require(type(seed) is int and 0 <= seed < (1 << 64), "seed must be a nonnegative u64")
    fmt.require(math.isfinite(label_mix) and 0 <= label_mix <= 1, "lambda must be in [0,1]")
    fmt.require(math.isfinite(k) and 0 < k <= 10, "K must be finite in (0,10]")
    fmt.require(threads is None or (type(threads) is int and 1 <= threads <= 256), "threads must be 1..256")


def load_manifest(dataset: Path, helper: Path) -> dict:
    """Validates the prepared dataset's provenance without reading its rows."""
    manifest = json.loads((dataset / "manifest.json").read_text())
    fmt.require(manifest["schema_version"] == 1 and manifest["architecture"] == data.ARCHITECTURE, "unsupported dataset schema")
    fmt.require(manifest["helper_sha256"] == data.sha256(helper), "feature/scoring helper differs from prepared dataset")
    for name in ("training", "development"):
        record = manifest["splits"][name]
        fmt.require(record["prepared_file"] == name + ".tsv", "unsafe prepared filename")
        fmt.require(data.sha256(dataset / record["prepared_file"]) == record["prepared_sha256"], f"{name} dataset hash mismatch")
    return manifest


def train(dataset: Path, helper: Path, output: Path, *, epochs: int = 10, batch_size: int = 256,
          rate: float = 0.001, l2: float = 1e-6, seed: int = 75,
          label_mix: float = 0.5, k: float = DEFAULT_K, threads: int | None = None) -> dict:
    validate_options(epochs, batch_size, rate, l2, seed, label_mix, k, threads)
    source_hashes = tool_source_hashes()
    fmt.require(source_hashes == LOADED_TOOL_HASHES, "tool sources changed since import; restart the trainer")
    helper = helper.resolve(strict=True)
    dataset = dataset.resolve(strict=True)
    output = output.absolute()
    fmt.require(not output.exists(), "output already exists; choose a new directory")
    inputs = [helper, dataset / "manifest.json", dataset / "training.tsv", dataset / "development.tsv"]
    hashes = {str(path): data.sha256(path) for path in inputs}
    load_manifest(dataset, helper)
    development = data.read_records(dataset / "development.tsv")
    hyperparameters = {"epochs": epochs, "batch_size": batch_size, "rate": rate, "l2": l2,
                       "seed": seed, "lambda": label_mix, "k": k}
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".nnue-train-", dir=output.parent) as temporary:
        staging = Path(temporary)
        model = staging / "model"
        command = [str(helper), "train", "--data-dir", str(dataset), "--output-dir", str(model),
                   "--epochs", str(epochs), "--batch-size", str(batch_size), "--rate", repr(rate),
                   "--l2", repr(l2), "--seed", str(seed), "--lambda", repr(label_mix), "--k", repr(k)]
        if threads is not None:
            command += ["--threads", str(threads)]
        log = staging / "training.log"
        with log.open("wb") as stream:
            result = subprocess.run(command, stdout=stream, stderr=subprocess.PIPE)
        fmt.require(result.returncode == 0, "Rust training failed: " + result.stderr.decode(errors="replace")[-3000:])
        training = json.loads((model / "training.json").read_text())
        for record in training["history"]:
            print(json.dumps(record, sort_keys=True), flush=True)
        (model / "network.nnue").rename(staging / "network.nnue")
        selected = next(record for record in training["history"] if record["epoch"] == training["selected_epoch"])
        parity = verify_rust(helper, staging / "network.nnue", parity_sample(development), staging)
        (staging / "parity.json").write_text(json.dumps(parity, indent=2) + "\n")
        report = {"schema_version": 2, "architecture": data.ARCHITECTURE, "training_completed": True,
                  "optimizer": f"full-parameter Adam, float32, {training['shards']} fixed gradient shards, "
                               f"{training['threads']} threads, SplitMix64 minibatches (nnue-data train)",
                  "steps": training["steps"], "hyperparameters": hyperparameters,
                  "inputs": hashes, "network_sha256": data.sha256(staging / "network.nnue"),
                  "unsupported_feature_rows": training["unsupported_feature_rows"],
                  "training_rows": training["training_rows"], "development_rows": training["development_rows"],
                  "training_metric_rows": training["training_metric_rows"],
                  "initial_integer_training": training["initial_integer_training"],
                  "initial_integer_development": training["initial_integer_development"],
                  "selected_epoch": training["selected_epoch"], "selected_metrics": selected,
                  "history": training["history"],
                  "parity_positions": parity["positions"], "python": platform.python_version(),
                  "platform": platform.platform(),
                  "tool_sha256": source_hashes,
                  "limitations": ["No Elo, speed or personality claim; external experimental model only.",
                                  "Development loss selects an epoch, not an independent strength confirmation.",
                                  "Split-overlap rejection does not establish opening-family or game independence.",
                                  "Exact Rust parity is checked on the recorded deterministic development subset, not every dataset row.",
                                  "Integer training metrics use a strided subsample of at most training_metric_rows positions."]}
        fmt.require(hashes == {str(path): data.sha256(path) for path in inputs}, "training inputs changed")
        fmt.require(tool_source_hashes() == source_hashes, "tool sources changed during training")
        (staging / "report.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
        log.rename(staging / "training.log.txt")
        (model / "training.json").unlink()
        model.rmdir()
        fmt.require(not output.exists(), "output appeared during training")
        staging.rename(output)
    return report


def arguments(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    subcommands = parser.add_subparsers(dest="command", required=True)
    prepare_parser = subcommands.add_parser("prepare")
    prepare_parser.add_argument("--helper", required=True, type=Path)
    prepare_parser.add_argument("--training", required=True, type=Path)
    prepare_parser.add_argument("--development", required=True, type=Path)
    prepare_parser.add_argument("--output-dir", required=True, type=Path)
    prepare_parser.add_argument("--deduplicate", action="store_true")
    prepare_parser.add_argument("--drop-development-overlap", action="store_true",
                                help="drop development rows that duplicate a training position instead of rejecting the split")
    trainer = subcommands.add_parser("train")
    trainer.add_argument("--helper", required=True, type=Path)
    trainer.add_argument("--data-dir", required=True, type=Path)
    trainer.add_argument("--output-dir", required=True, type=Path)
    trainer.add_argument("--epochs", type=int, default=10)
    trainer.add_argument("--batch-size", type=int, default=256)
    trainer.add_argument("--rate", type=float, default=0.001)
    trainer.add_argument("--l2", type=float, default=1e-6)
    trainer.add_argument("--seed", type=int, default=75)
    trainer.add_argument("--lambda", dest="label_mix", type=float, default=0.5)
    trainer.add_argument("--k", type=float, default=DEFAULT_K)
    trainer.add_argument("--threads", type=int, help="trainer threads; the result does not depend on it")
    return parser.parse_args(argv)


def main(argv=None) -> int:
    options = arguments(argv)
    try:
        if options.command == "prepare":
            report = data.prepare(options.helper, options.training, options.development, options.output_dir,
                                  options.deduplicate, options.drop_development_overlap)
            print(json.dumps({name: {key: value for key, value in record.items() if key != "feature_support"}
                              for name, record in report["splits"].items()}, indent=2))
        else:
            report = train(options.data_dir, options.helper, options.output_dir, epochs=options.epochs,
                           batch_size=options.batch_size, rate=options.rate, l2=options.l2,
                           seed=options.seed, label_mix=options.label_mix, k=options.k, threads=options.threads)
            print(json.dumps({"network_sha256": report["network_sha256"], "selected_epoch": report["selected_epoch"],
                              "parity_positions": report["parity_positions"], "output": str(options.output_dir)}, indent=2))
        return 0
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f"train_nnue: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
