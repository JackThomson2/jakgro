#!/usr/bin/env python3
"""Prepare, train and verify external experimental Jakgro NNUE networks on CPU.

Training requires NumPy (see requirements-nnue.txt); preparation and scalar
integer verification use the standard library. This tool never changes engine
configuration or installs a model. Each output is a new, completed directory.
"""

from __future__ import annotations

import argparse
from array import array
import json
import math
import os
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

# Limit CPU BLAS scheduling before the lazy NumPy import. Floating results are
# reproducible on the recorded NumPy/platform combination, not across all BLAS.
for _variable in ("OPENBLAS_NUM_THREADS", "MKL_NUM_THREADS", "OMP_NUM_THREADS", "VECLIB_MAXIMUM_THREADS"):
    os.environ[_variable] = "1"

DEFAULT_K = 0.8806824810924139
FLOAT_CP_SCALE = 400.0


def tool_source_hashes() -> dict[str, str]:
    paths = (Path(__file__), Path(data.__file__), Path(fmt.__file__))
    return {path.name: data.sha256(path) for path in paths}


LOADED_TOOL_HASHES = tool_source_hashes()


def numpy():
    try:
        import numpy as np
    except ImportError as error:
        raise ValueError("CPU training requires NumPy; install tools/requirements-nnue.txt") from error
    return np


def encode_samples(samples: list[data.Sample], label_mix: float, k: float) -> dict:
    np = numpy()
    indices = np.full((len(samples), 2, 64), fmt.INPUTS, dtype=np.int32)
    outcomes, teachers, present = [], [], []
    for row, sample in enumerate(samples):
        views = (sample.white, sample.black) if sample.stm == 0 else (sample.black, sample.white)
        for side, view in enumerate(views):
            indices[row, side, :len(view)] = view
        outcomes.append(sample.outcome if sample.stm == 0 else 1.0 - sample.outcome)
        teachers.append((sample.teacher or 0.0) * (1 if sample.stm == 0 else -1))
        present.append(sample.teacher is not None)
    outcomes = np.asarray(outcomes, dtype=np.float64)
    teacher_probability = probability(np.asarray(teachers), k)
    labels = np.where(present, label_mix * outcomes + (1.0 - label_mix) * teacher_probability, outcomes)
    return {"indices": indices, "outcomes": outcomes, "labels": labels}


def probability(scores, k: float):
    np = numpy()
    exponent = np.clip(-scores * (math.log(10.0) * k / 400.0), -80.0, 80.0)
    return 1.0 / (1.0 + np.exp(exponent))


def initialize(support: list[int], seed: int) -> dict:
    np = numpy()
    generator = np.random.Generator(np.random.PCG64(seed))
    inputs = generator.normal(0.0, 0.02, (fmt.INPUTS + 1, fmt.HIDDEN))
    supported = np.asarray(support) > 0
    inputs[:fmt.INPUTS][~supported] = 0.0
    inputs[fmt.INPUTS] = 0.0  # Padding is never a feature.
    return {"input": inputs, "hidden": np.full(fmt.HIDDEN, 0.1),
            "output": generator.normal(0.0, 0.01, (2, fmt.HIDDEN)),
            "bias": np.zeros(1)}


def forward(parameters: dict, indices):
    np = numpy()
    sums = parameters["hidden"] + parameters["input"][indices].sum(axis=2)
    activations = np.clip(sums, 0.0, 1.0)
    raw_scores = FLOAT_CP_SCALE * ((activations * parameters["output"]).sum(axis=(1, 2)) + parameters["bias"][0])
    return np.clip(raw_scores, -fmt.MAX_SCORE, fmt.MAX_SCORE), sums, activations, raw_scores


def gradients(parameters: dict, indices, labels, k: float, l2: float) -> tuple[dict, float]:
    np = numpy()
    scores, sums, activations, raw_scores = forward(parameters, indices)
    predictions = probability(scores, k)
    loss = float(np.mean((predictions - labels) ** 2))
    derivative = (2.0 / len(labels)) * (predictions - labels) * predictions * (1.0 - predictions)
    derivative *= math.log(10.0) * k / 400.0
    derivative *= (raw_scores > -fmt.MAX_SCORE) & (raw_scores < fmt.MAX_SCORE)
    outer = derivative * FLOAT_CP_SCALE
    hidden_gradient = outer[:, None, None] * parameters["output"] * ((sums > 0.0) & (sums < 1.0))
    input_gradient = np.zeros_like(parameters["input"])
    # add.at is essential: repeated feature rows within a batch must accumulate.
    for side in range(2):
        np.add.at(input_gradient, indices[:, side, :], hidden_gradient[:, side, None, :])
    input_gradient[fmt.INPUTS] = 0.0
    result = {"input": input_gradient,
              "hidden": hidden_gradient.sum(axis=(0, 1)),
              "output": (outer[:, None, None] * activations).sum(axis=0),
              "bias": np.asarray([outer.sum()])}
    for name, gradient in result.items():
        gradient += l2 * parameters[name]
    result["input"][fmt.INPUTS] = 0.0
    return result, loss


def quantize(values, scale: float, low: int, high: int):
    np = numpy()
    scaled = np.asarray(values, dtype=np.float64) * scale
    fmt.require(bool(np.isfinite(scaled).all()), "nonfinite parameter during export")
    rounded = np.copysign(np.floor(np.abs(scaled) + 0.5), scaled)
    fmt.require(bool(((rounded >= low) & (rounded <= high)).all()), "quantized parameter exceeds storage bounds")
    return rounded.astype(np.int64)


def export_network(parameters: dict) -> fmt.Network:
    hidden = quantize(parameters["hidden"], fmt.ACTIVATION, -32768, 32767)
    inputs = quantize(parameters["input"][:fmt.INPUTS], fmt.ACTIVATION, -32768, 32767)
    output = quantize(parameters["output"], FLOAT_CP_SCALE * fmt.OUTPUT_SCALE, -32768, 32767)
    bias = quantize(parameters["bias"], FLOAT_CP_SCALE * fmt.CP_DIVISOR, -(1 << 31), (1 << 31) - 1)
    network = fmt.Network(tuple(map(int, hidden)), array("h", map(int, inputs.ravel())),
                          tuple(map(int, output.ravel())), int(bias[0]))
    network.validate()
    return network


def integer_predictions(network: fmt.Network, encoded: dict, batch_size: int = 256):
    np = numpy()
    network.validate()
    inputs = np.zeros((fmt.INPUTS + 1, fmt.HIDDEN), dtype=np.int64)
    inputs[:fmt.INPUTS] = np.asarray(network.input_weights, dtype=np.int64).reshape(fmt.INPUTS, fmt.HIDDEN)
    hidden = np.asarray(network.hidden_bias, dtype=np.int64)
    output = np.asarray(network.output_weights, dtype=np.int64).reshape(2, fmt.HIDDEN)
    scores = []
    for start in range(0, len(encoded["indices"]), batch_size):
        indices = encoded["indices"][start:start + batch_size]
        sums = hidden + inputs[indices].sum(axis=2)
        numerator = network.output_bias + (np.clip(sums, 0, fmt.ACTIVATION) * output).sum(axis=(1, 2))
        cp = np.sign(numerator) * (np.abs(numerator) // fmt.CP_DIVISOR)
        scores.append(np.clip(cp, -fmt.MAX_SCORE, fmt.MAX_SCORE))
    return np.concatenate(scores)


def metrics(scores, encoded: dict, k: float) -> dict:
    np = numpy()
    predictions = probability(scores, k)
    return {"rows": len(scores), "label_mse": float(np.mean((predictions - encoded["labels"]) ** 2)),
            "outcome_mse": float(np.mean((predictions - encoded["outcomes"]) ** 2)),
            "minimum_cp": float(np.min(scores)), "maximum_cp": float(np.max(scores))}


def float_metrics(parameters: dict, encoded: dict, k: float) -> dict:
    np = numpy()
    scores = [forward(parameters, encoded["indices"][start:start + 256])[0]
              for start in range(0, len(encoded["indices"]), 256)]
    return metrics(np.concatenate(scores), encoded, k)


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


def validate_options(epochs: int, batch_size: int, rate: float, l2: float, seed: int, label_mix: float, k: float) -> None:
    fmt.require(type(epochs) is int and 1 <= epochs <= 10000, "epochs must be 1..10000; initialization is not a trained model")
    fmt.require(type(batch_size) is int and 1 <= batch_size <= 2048, "batch size must be 1..2048")
    fmt.require(math.isfinite(rate) and 0 < rate <= 1, "rate must be finite in (0,1]")
    fmt.require(math.isfinite(l2) and 0 <= l2 <= 1, "L2 must be finite in [0,1]")
    fmt.require(type(seed) is int and 0 <= seed < (1 << 64), "seed must be a nonnegative u64")
    fmt.require(math.isfinite(label_mix) and 0 <= label_mix <= 1, "lambda must be in [0,1]")
    fmt.require(math.isfinite(k) and 0 < k <= 10, "K must be finite in (0,10]")


def train(dataset: Path, helper: Path, output: Path, *, epochs: int = 10, batch_size: int = 256,
          rate: float = 0.001, l2: float = 1e-6, seed: int = 75,
          label_mix: float = 0.5, k: float = DEFAULT_K) -> dict:
    validate_options(epochs, batch_size, rate, l2, seed, label_mix, k)
    source_hashes = tool_source_hashes()
    fmt.require(source_hashes == LOADED_TOOL_HASHES, "tool sources changed since import; restart the trainer")
    np = numpy()
    helper = helper.resolve(strict=True)
    dataset = dataset.resolve(strict=True)
    output = output.absolute()
    fmt.require(not output.exists(), "output already exists; choose a new directory")
    inputs = [helper, dataset / "manifest.json", dataset / "training.tsv", dataset / "development.tsv"]
    hashes = {str(path): data.sha256(path) for path in inputs}
    manifest, training, development = data.load_dataset(dataset, helper)
    train_rows, dev_rows = (encode_samples(rows, label_mix, k) for rows in (training, development))
    support = manifest["splits"]["training"]["feature_support"]
    parameters = initialize(support, seed)
    initial = export_network(parameters)
    initial_train = metrics(integer_predictions(initial, train_rows), train_rows, k)
    initial_dev = metrics(integer_predictions(initial, dev_rows), dev_rows, k)
    moment = {name: np.zeros_like(value) for name, value in parameters.items()}
    velocity = {name: np.zeros_like(value) for name, value in parameters.items()}
    generator = np.random.Generator(np.random.PCG64(seed ^ 0x9E3779B97F4A7C15))
    step = 0
    history = []
    best = None
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".nnue-train-", dir=output.parent) as temporary:
        staging = Path(temporary)
        for epoch in range(1, epochs + 1):
            order = generator.permutation(len(training))
            for start in range(0, len(order), batch_size):
                batch = order[start:start + batch_size]
                gradient, loss = gradients(parameters, train_rows["indices"][batch], train_rows["labels"][batch], k, l2)
                fmt.require(math.isfinite(loss), "nonfinite training loss")
                step += 1
                for name in parameters:
                    fmt.require(bool(np.isfinite(gradient[name]).all()), "nonfinite training gradient")
                    moment[name] *= 0.9
                    moment[name] += 0.1 * gradient[name]
                    velocity[name] *= 0.999
                    velocity[name] += 0.001 * gradient[name] ** 2
                    parameters[name] -= rate * (moment[name] / (1 - 0.9 ** step)) / (np.sqrt(velocity[name] / (1 - 0.999 ** step)) + 1e-8)
                parameters["input"][fmt.INPUTS] = 0.0
            network = export_network(parameters)
            train_metric = metrics(integer_predictions(network, train_rows), train_rows, k)
            dev_metric = metrics(integer_predictions(network, dev_rows), dev_rows, k)
            record = {"epoch": epoch, "steps": step, "integer_training": train_metric, "integer_development": dev_metric,
                      "float_training": float_metrics(parameters, train_rows, k),
                      "float_development": float_metrics(parameters, dev_rows, k)}
            history.append(record)
            print(json.dumps(record, sort_keys=True), flush=True)
            # A trained epoch must improve the emitted training target. Of those,
            # select by emitted development outcome loss, never floating loss.
            if train_metric["label_mse"] < initial_train["label_mse"] and (best is None or dev_metric["outcome_mse"] < best["integer_development"]["outcome_mse"]):
                best = record
                (staging / "network.nnue").write_bytes(network.to_bytes())
        fmt.require(best is not None, "no trained integer model improved training loss; no artifact exported")
        exported = fmt.Network.from_bytes((staging / "network.nnue").read_bytes())
        unsupported = np.asarray(support) == 0
        weights = np.asarray(exported.input_weights).reshape(fmt.INPUTS, fmt.HIDDEN)
        fmt.require(not np.any(weights[unsupported]), "unobserved feature rows must remain zero")
        parity = verify_rust(helper, staging / "network.nnue", parity_sample(training) + parity_sample(development), staging)
        (staging / "parity.json").write_text(json.dumps(parity, indent=2) + "\n")
        report = {"schema_version": 1, "architecture": data.ARCHITECTURE, "training_completed": True,
                  "optimizer": "full-parameter Adam, float64, PCG64 minibatches", "steps": step,
                  "hyperparameters": {"epochs": epochs, "batch_size": batch_size, "rate": rate, "l2": l2,
                                      "seed": seed, "lambda": label_mix, "k": k},
                  "inputs": hashes, "network_sha256": data.sha256(staging / "network.nnue"),
                  "unsupported_feature_rows": int(unsupported.sum()),
                  "initial_integer_training": initial_train, "initial_integer_development": initial_dev,
                  "selected_epoch": best["epoch"], "selected_metrics": best, "history": history,
                  "parity_positions": parity["positions"], "numpy": np.__version__, "python": platform.python_version(),
                  "platform": platform.platform(), "blas_threads": 1,
                  "tool_sha256": source_hashes,
                  "limitations": ["No Elo, speed or personality claim; external experimental model only.",
                                  "Development loss selects an epoch, not an independent strength confirmation.",
                                  "Split-overlap rejection does not establish opening-family or game independence.",
                                  "Exact Rust parity is checked on the recorded deterministic subset, not every dataset row."]}
        fmt.require(hashes == {str(path): data.sha256(path) for path in inputs}, "training inputs changed")
        fmt.require(tool_source_hashes() == source_hashes, "tool sources changed during training")
        (staging / "report.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
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
    return parser.parse_args(argv)


def main(argv=None) -> int:
    options = arguments(argv)
    try:
        if options.command == "prepare":
            report = data.prepare(options.helper, options.training, options.development, options.output_dir, options.deduplicate)
            print(json.dumps({name: {key: value for key, value in record.items() if key != "feature_support"}
                              for name, record in report["splits"].items()}, indent=2))
        else:
            report = train(options.data_dir, options.helper, options.output_dir, epochs=options.epochs,
                           batch_size=options.batch_size, rate=options.rate, l2=options.l2,
                           seed=options.seed, label_mix=options.label_mix, k=options.k)
            print(json.dumps({"network_sha256": report["network_sha256"], "selected_epoch": report["selected_epoch"],
                              "parity_positions": report["parity_positions"], "output": str(options.output_dir)}, indent=2))
        return 0
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f"train_nnue: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
