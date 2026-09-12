#!/usr/bin/env python3
"""Build and validate an opt-in, host-native PGO engine in a new output directory."""

from __future__ import annotations

import argparse
import dataclasses
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any

try:
    from . import measure_style
except ImportError:
    import measure_style

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_TRAINING = (Path("tools/data/openings.epd"), Path("tools/data/pgo-training.epd"))
DEFAULT_VALIDATION = Path("tests/data/search-performance.epd")


class PgoEngine(measure_style.UciEngine):
    """Retain the search transcript for deterministic PV validation."""

    def read_until(self, predicate) -> list[str]:
        self.last_response = super().read_until(predicate)
        return self.last_response


def search_iterations(lines: list[str]) -> list[dict[str, Any]]:
    iterations = []
    for line in lines:
        info = measure_style.parse_search_info(line)
        if info is not None:
            score, depth, nodes, _, _ = info
            iterations.append({"score": score, "depth": depth, "nodes": nodes,
                               "pv": line.partition(" pv ")[2].split()})
    return iterations



def positive(value: str) -> int:
    number = int(value)
    if number <= 0:
        raise argparse.ArgumentTypeError("value must be positive")
    return number


def profiles(value: str) -> list[int]:
    try:
        result = [int(part) for part in value.split(",")]
    except ValueError as error:
        raise argparse.ArgumentTypeError("profiles must be comma-separated integers") from error
    if not result or len(set(result)) != len(result) or any(not 0 <= item <= 100 for item in result):
        raise argparse.ArgumentTypeError("profiles must be unique values between 0 and 100")
    return result


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def write_json(path: Path, payload: Any) -> None:
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    temporary.replace(path)


def load_suite(paths: list[Path], nodes: int) -> list[measure_style.Fixture]:
    fixtures = []
    identifiers: set[str] = set()
    positions: set[str] = set()
    for path in paths:
        for line_number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            line = raw.strip()
            if not line or line.startswith("#"):
                continue
            fields = re.split(r";\s*id\s+|\s+id\s+", line, maxsplit=1)
            if len(fields) != 2 or ";" not in fields[1]:
                raise ValueError(f"{path}:{line_number}: missing terminated id field")
            fen = fields[0].rstrip("; ")
            tokens = fen.split()
            identifier = fields[1].split(";", 1)[0].strip()
            if identifier.startswith('"') and identifier.endswith('"'):
                identifier = identifier[1:-1]
            if not identifier or '"' in identifier:
                raise ValueError(f"{path}:{line_number}: invalid id")
            if len(tokens) not in (4, 6) or tokens[1] not in ("w", "b"):
                raise ValueError(f"{path}:{line_number}: expected a four- or six-field position")
            if len(tokens) == 6 and (not tokens[4].isdigit() or not tokens[5].isdigit() or int(tokens[5]) < 1):
                raise ValueError(f"{path}:{line_number}: invalid FEN counters")
            position = " ".join(tokens[:4])
            if identifier in identifiers or position in positions:
                raise ValueError(f"{path}:{line_number}: duplicate id or position")
            identifiers.add(identifier)
            positions.add(position)
            fixtures.append(measure_style.Fixture(identifier, "pgo", fen if len(tokens) == 6 else fen + " 0 1", nodes, {}))
    if not fixtures:
        raise ValueError("suite contains no positions")
    return fixtures


def ensure_disjoint(training: list[measure_style.Fixture], validation: list[measure_style.Fixture]) -> None:
    starts = {" ".join(fixture.fen.split()[:4]) for fixture in training}
    if any(" ".join(fixture.fen.split()[:4]) in starts for fixture in validation):
        raise ValueError("training and validation contain the same starting position")


def source_inputs(root: Path) -> dict[str, str]:
    paths = [root / "Cargo.toml", root / "Cargo.lock"]
    paths.extend(sorted((root / "src").rglob("*.rs")))
    for name in ["build.rs", ".cargo/config", ".cargo/config.toml", "tools/build_pgo.py", "tools/measure_style.py"]:
        path = root / name
        if path.is_file():
            paths.append(path)
    return {str(path.relative_to(root)): sha256(path) for path in paths}


def command_text(command: list[str], root: Path) -> str:
    return subprocess.check_output(
        command, cwd=root, text=True, stderr=subprocess.STDOUT, timeout=30
    ).strip()


def llvm_major(version: str) -> int:
    match = re.search(r"LLVM version:\s*(\d+)|LLVM version\s+(\d+)", version, re.IGNORECASE)
    if match is None:
        raise ValueError("cannot determine LLVM version")
    return int(next(group for group in match.groups() if group is not None))


def toolchain(root: Path, explicit_profdata: Path | None) -> dict[str, str]:
    cargo = shutil.which("cargo")
    rustc = shutil.which("rustc")
    if cargo is None or rustc is None:
        raise ValueError("cargo and rustc must be on PATH")
    rust_version = command_text([rustc, "--version", "--verbose"], root)
    host_match = re.search(r"^host: (\S+)$", rust_version, re.MULTILINE)
    if host_match is None:
        raise ValueError("rustc did not report a host triple")
    host = host_match.group(1)
    sysroot = Path(command_text([rustc, "--print", "sysroot"], root))
    suffix = ".exe" if os.name == "nt" else ""
    profdata = (explicit_profdata or sysroot / "lib/rustlib" / host / "bin" / ("llvm-profdata" + suffix)).resolve()
    if not profdata.is_file():
        raise ValueError("matching llvm-profdata is missing; install rustup component add llvm-tools-preview or pass --llvm-profdata")
    llvm_version = command_text([str(profdata), "--version"], root)
    if llvm_major(rust_version) != llvm_major(llvm_version):
        raise ValueError("llvm-profdata must match rustc's LLVM major version")
    return {"cargo": cargo, "rustc": rustc, "host": host, "rustc_version": rust_version,
            "cargo_version": command_text([cargo, "--version"], root),
            "llvm_profdata": str(profdata), "llvm_profdata_version": llvm_version,
            "llvm_profdata_sha256": sha256(profdata)}


def build_environment(flags: list[str], environment: dict[str, str]) -> dict[str, str]:
    overrides = [
        "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTC", "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER", "CARGO_BUILD_RUSTC", "CARGO_BUILD_RUSTC_WRAPPER",
        "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER",
    ]
    conflicts = [name for name in overrides if environment.get(name)]
    if conflicts:
        raise ValueError("unset build overrides for this controlled build: " + ", ".join(conflicts))
    result = dict(environment)
    result.pop("LLVM_PROFILE_FILE", None)
    result["CARGO_ENCODED_RUSTFLAGS"] = "\x1f".join(flags)
    result["CARGO_INCREMENTAL"] = "0"
    return result


def run_command(command: list[str], root: Path, environment: dict[str, str], log: Path, timeout: int) -> None:
    with log.open("w", encoding="utf-8") as output:
        output.write(json.dumps(command) + "\n")
        output.flush()
        completed = subprocess.run(command, cwd=root, env=environment, stdout=output, stderr=subprocess.STDOUT, timeout=timeout, check=False)
    if completed.returncode != 0:
        raise RuntimeError(f"command failed with status {completed.returncode}; see {log}")


def build_stage(root: Path, out: Path, stage: str, flags: list[str], compiler: dict[str, str], timeout: int, manifest: dict[str, Any]) -> Path:
    target_dir = out / "build" / stage
    command = [compiler["cargo"], "build", "--release", "--locked", "--target", compiler["host"],
               "--target-dir", str(target_dir), "--bin", "jakgro"]
    environment = build_environment(flags, dict(os.environ))
    manifest["commands"].append({"stage": stage, "argv": command, "rustflags": flags, "log": stage + ".log"})
    run_command(command, root, environment, out / (stage + ".log"), timeout)
    binary = target_dir / compiler["host"] / "release" / ("jakgro.exe" if os.name == "nt" else "jakgro")
    manifest["binaries"][stage] = {"path": str(binary), "sha256": sha256(binary)}
    return binary


def measure(binary: Path, fixtures: list[measure_style.Fixture], selected_profiles: list[int], timeout: int, hash_mib: int) -> list[dict[str, Any]]:
    observations = []
    with PgoEngine(binary, timeout) as engine:
        engine.send("setoption name Threads value 1")
        engine.send(f"setoption name Hash value {hash_mib}")
        engine.send("isready")
        engine.read_until(lambda line: line == "readyok")
        for profile in selected_profiles:
            for fixture in fixtures:
                observed = engine.measure(fixture, profile)
                iterations = search_iterations(engine.last_response)
                if not iterations or iterations[-1]["pv"][:1] != [observed.bestmove]:
                    raise ValueError(f"{fixture.identifier}: missing or inconsistent principal variation")
                observations.append({"id": fixture.identifier, "fen": fixture.fen, "aggression": profile,
                                     "requested_nodes": fixture.nodes, "iterations": iterations,
                                     **dataclasses.asdict(observed)})
    if engine.process.returncode != 0:
        raise RuntimeError("training/validation engine did not exit cleanly; profile may be incomplete")
    return observations


def validate_observations(reference: list[dict[str, Any]], candidate: list[dict[str, Any]]) -> None:
    if not reference or len(reference) != len(candidate):
        raise ValueError("validation observation counts differ or are empty")
    keys = ("id", "fen", "aggression", "requested_nodes", "bestmove", "score", "depth", "nodes", "personality", "iterations")
    for before, after in zip(reference, candidate):
        if any(before[key] != after[key] for key in keys):
            raise ValueError(f"PGO changed fixed-node search: {before['id']} at Aggression {before['aggression']}")


def parse_arguments(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, required=True, help="new directory for binaries, profiles, logs and manifest; never reused")
    parser.add_argument("--llvm-profdata", type=Path, help="matching Rust LLVM tool; defaults to the llvm-tools-preview component")
    parser.add_argument("--training-suite", type=Path, action="append", help="repeat for multiple EPD/FEN suites; defaults to curated openings plus pgo-training.epd")
    parser.add_argument("--validation-suite", type=Path, default=DEFAULT_VALIDATION)
    parser.add_argument("--nodes", type=positive, default=250_000, help="fixed nodes per training position/profile")
    parser.add_argument("--validation-nodes", type=positive, default=100_000)
    parser.add_argument("--profiles", type=profiles, default=profiles("0,75,100"))
    parser.add_argument("--hash", type=positive, default=16)
    parser.add_argument("--timeout", type=positive, default=120, help="seconds per UCI response")
    parser.add_argument("--build-timeout", type=positive, default=600, help="seconds per build/merge command")
    parser.add_argument("--target-cpu", help="optional CPU specialization, e.g. native; omitted keeps portable target defaults")
    parser.add_argument("--revision", help="optional user-supplied source revision label; source hashes are always recorded")
    args = parser.parse_args(argv)
    if args.hash > 1024:
        parser.error("--hash must be at most 1024 MiB")
    if args.target_cpu and re.fullmatch(r"[A-Za-z0-9_.+-]+", args.target_cpu) is None:
        parser.error("invalid --target-cpu")
    return args


def build(args: argparse.Namespace, root: Path = ROOT) -> Path:
    root = root.resolve()
    out = args.output_dir.resolve()
    if out.exists():
        raise ValueError("output directory already exists; choose a new directory")
    build_environment([], dict(os.environ))
    training_paths = [(root / path).resolve() for path in (args.training_suite or DEFAULT_TRAINING)]
    validation_path = (root / args.validation_suite).resolve()
    training = load_suite(training_paths, args.nodes)
    validation = load_suite([validation_path], args.validation_nodes)
    ensure_disjoint(training, validation)
    compiler = toolchain(root, args.llvm_profdata)
    initial_sources = source_inputs(root)
    suites = {str(path): sha256(path) for path in training_paths + [validation_path]}
    manifest: dict[str, Any] = {
        "schema_version": 1, "status": "running", "source_root": str(root), "revision_label": args.revision,
        "source_inputs": initial_sources, "toolchain": compiler, "suite_sha256": suites,
        "settings": {"profiles": args.profiles, "training_nodes": args.nodes, "validation_nodes": args.validation_nodes,
                     "training_positions": len(training), "validation_positions": len(validation),
                     "threads": 1, "hash_mib": args.hash, "target_cpu": args.target_cpu,
                     "training_suites": [str(path) for path in training_paths], "validation_suite": str(validation_path)},
        "commands": [], "binaries": {},
        "build_environment": {name: value for name, value in os.environ.items() if name.startswith("CARGO_PROFILE_RELEASE_")},
    }
    out.mkdir(parents=True, exist_ok=False)
    write_json(out / "manifest.json", manifest)
    flags = [f"-Ctarget-cpu={args.target_cpu}"] if args.target_cpu else []
    try:
        baseline = build_stage(root, out, "baseline", flags, compiler, args.build_timeout, manifest)
        raw_dir = out / "profiles"
        raw_dir.mkdir()
        instrumented = build_stage(root, out, "instrumented", flags + [f"-Cprofile-generate={raw_dir}"], compiler, args.build_timeout, manifest)
        # The compiler's embedded profile directory is authoritative for the child.
        old_profile = os.environ.pop("LLVM_PROFILE_FILE", None)
        try:
            observations = measure(instrumented, training, args.profiles, args.timeout, args.hash)
        finally:
            if old_profile is not None:
                os.environ["LLVM_PROFILE_FILE"] = old_profile
        write_json(out / "training.json", observations)
        raw = sorted(raw_dir.glob("*.profraw"))
        if not raw or any(path.stat().st_size == 0 for path in raw):
            raise ValueError("instrumented training produced no usable raw profiles")
        manifest["raw_profiles"] = {path.name: sha256(path) for path in raw}
        profile = out / "merged.profdata"
        merge = [compiler["llvm_profdata"], "merge", "-o", str(profile), *map(str, raw)]
        manifest["commands"].append({"stage": "merge", "argv": merge, "log": "merge.log"})
        run_command(merge, root, dict(os.environ), out / "merge.log", args.build_timeout)
        if profile.stat().st_size == 0:
            raise ValueError("merged profile is empty")
        manifest["profile_sha256"] = sha256(profile)
        optimized = build_stage(root, out, "optimized", flags + [f"-Cprofile-use={profile}"], compiler, args.build_timeout, manifest)
        before = measure(baseline, validation, args.profiles, args.timeout, args.hash)
        after = measure(optimized, validation, args.profiles, args.timeout, args.hash)
        write_json(out / "validation.json", {"baseline": before, "optimized": after})
        validate_observations(before, after)
        if initial_sources != source_inputs(root) or any(sha256(Path(path)) != value for path, value in suites.items()):
            raise ValueError("source or suite inputs changed during the build")
        suffix = ".exe" if os.name == "nt" else ""
        published = out / ("jakgro-pgo" + suffix)
        shutil.copy2(baseline, out / ("jakgro-baseline" + suffix))
        shutil.copy2(optimized, published)
        manifest["output"] = {"path": str(published), "sha256": sha256(published)}
        manifest["training_sha256"] = sha256(out / "training.json")
        manifest["validation_sha256"] = sha256(out / "validation.json")
        manifest["status"] = "complete"
    except BaseException as error:
        manifest["status"] = "failed"
        manifest["error"] = f"{type(error).__name__}: {error}"
        write_json(out / "manifest.json", manifest)
        raise
    write_json(out / "manifest.json", manifest)
    return published


def main(argv: list[str] | None = None) -> int:
    try:
        output = build(parse_arguments(argv))
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"build_pgo: {error}", file=sys.stderr)
        return 1
    print(f"Validated PGO engine: {output}")
    print(f"Provenance: {output.parent / 'manifest.json'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
