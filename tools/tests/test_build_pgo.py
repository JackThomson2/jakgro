import argparse
import copy
import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from tools import build_pgo

START = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq -"
OTHER = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR b KQkq -"


def observation():
    return {
        "id": "position", "fen": START + " 0 1", "aggression": 75,
        "requested_nodes": 1000, "bestmove": "e2e4", "score": "cp 20",
        "depth": 3, "nodes": 900, "elapsed_ms": 5, "nps": 180000,
        "personality": {"attempts": 2},
        "iterations": [{"score": "cp 20", "depth": 3, "nodes": 900,
                        "pv": ["e2e4", "e7e5", "g1f3"]}],
    }


class InputTests(unittest.TestCase):
    def test_limits_and_profiles_are_validated(self):
        self.assertEqual(build_pgo.positive("1"), 1)
        self.assertEqual(build_pgo.profiles("0,75,100"), [0, 75, 100])
        for value in ["", "-1", "101", "75,75", "75,x"]:
            with self.subTest(value=value), self.assertRaises(argparse.ArgumentTypeError):
                build_pgo.profiles(value)
        with self.assertRaises(argparse.ArgumentTypeError):
            build_pgo.positive("0")

    def test_defaults_keep_the_portable_cpu_and_three_profiles(self):
        args = build_pgo.parse_arguments(["--output-dir", "fresh"])
        self.assertIsNone(args.target_cpu)
        self.assertEqual(args.profiles, [0, 75, 100])
        self.assertEqual(args.nodes, 250000)

    def test_cli_rejects_invalid_resource_settings(self):
        for extra in [["--nodes", "0"], ["--hash", "1025"], ["--target-cpu", "native -Cbad"]]:
            with self.subTest(extra=extra), mock.patch("sys.stderr"), self.assertRaises(SystemExit):
                build_pgo.parse_arguments(["--output-dir", "fresh", *extra])

    def test_four_and_six_field_suites_preserve_counters_and_ignore_other_operations(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "suite.epd"
            path.write_text(f'# comment\n{START} id "first";\n{OTHER} 12 7 ; id second ; nodes 25 ; bm0 a2a4\n')
            fixtures = build_pgo.load_suite([path], 300)
        self.assertEqual([fixture.identifier for fixture in fixtures], ["first", "second"])
        self.assertEqual(fixtures[0].fen, START + " 0 1")
        self.assertEqual(fixtures[1].fen, OTHER + " 12 7")
        self.assertTrue(all(fixture.nodes == 300 for fixture in fixtures))

    def test_invalid_or_duplicate_records_are_refused(self):
        invalid = [
            "# only comments\n", f'{START} id "unterminated"',
            f'{START} 0 0 ; id bad ;', f'{START} -1 1 ; id bad ;',
            f'{START} id "a";\n{START} 2 3 ; id b ;',
            f'{START} id "a";\n{OTHER} id "a";',
            '8/8/8/8/8/8/8/8 x - - id "bad";',
        ]
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "suite.epd"
            for text in invalid:
                with self.subTest(text=text):
                    path.write_text(text)
                    with self.assertRaises(ValueError):
                        build_pgo.load_suite([path], 1000)

    def test_disjointness_ignores_move_counters(self):
        first = build_pgo.measure_style.Fixture("first", "pgo", START + " 0 1", 1, {})
        second = build_pgo.measure_style.Fixture("second", "pgo", START + " 4 8", 1, {})
        with self.assertRaisesRegex(ValueError, "same starting"):
            build_pgo.ensure_disjoint([first], [second])

    def test_shipped_training_and_validation_starts_are_separate(self):
        root = build_pgo.ROOT
        training = build_pgo.load_suite([root / path for path in build_pgo.DEFAULT_TRAINING], 1000)
        validation = build_pgo.load_suite([root / build_pgo.DEFAULT_VALIDATION], 1000)
        self.assertEqual(len(training), 56)
        self.assertEqual(len(validation), 10)
        build_pgo.ensure_disjoint(training, validation)


class BuildEnvironmentTests(unittest.TestCase):
    def test_encoded_flags_preserve_spaces_without_mutating_the_parent(self):
        parent = {"PATH": "/bin", "LLVM_PROFILE_FILE": "/old/profile", "CARGO_INCREMENTAL": "1"}
        original = parent.copy()
        flags = ["-Cprofile-generate=/new output/profiles", "-Ctarget-cpu=native"]
        result = build_pgo.build_environment(flags, parent)
        self.assertEqual(parent, original)
        self.assertNotIn("LLVM_PROFILE_FILE", result)
        self.assertEqual(result["CARGO_ENCODED_RUSTFLAGS"].split("\x1f"), flags)
        self.assertEqual(result["CARGO_INCREMENTAL"], "0")

    def test_ambient_compiler_and_wrapper_overrides_are_refused(self):
        for name in ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTC", "RUSTC_WRAPPER",
                     "RUSTC_WORKSPACE_WRAPPER", "CARGO_BUILD_RUSTC", "CARGO_BUILD_RUSTC_WRAPPER",
                     "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER"]:
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, "unset build overrides"):
                build_pgo.build_environment([], {name: "value"})

    def test_matching_llvm_version_formats_are_recognized(self):
        self.assertEqual(build_pgo.llvm_major("LLVM version: 20.1.5"), 20)
        self.assertEqual(build_pgo.llvm_major("LLVM version 20.1.5-rust-1.88.0-stable"), 20)
        with self.assertRaises(ValueError):
            build_pgo.llvm_major("unknown tool")

    def test_mismatched_profdata_is_rejected_before_building(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            profdata = root / "llvm-profdata"
            profdata.write_bytes(b"tool")
            responses = ["rustc 1.88.0\nhost: x86_64-unknown-linux-gnu\nLLVM version: 20.1.5",
                         str(root), "LLVM version 18.1.3"]
            with mock.patch.object(build_pgo.shutil, "which", side_effect=lambda name: "/bin/" + name), \
                 mock.patch.object(build_pgo, "command_text", side_effect=responses):
                with self.assertRaisesRegex(ValueError, "LLVM major"):
                    build_pgo.toolchain(root, profdata)

    def test_stages_use_separate_directories_and_explicit_target(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            out = root / "output with spaces"
            out.mkdir()
            compiler = {"cargo": "cargo", "host": "test-host"}
            manifest = {"commands": [], "binaries": {}}
            suffix = ".exe" if os.name == "nt" else ""
            binary = out / "build/optimized/test-host/release" / ("jakgro" + suffix)
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"engine")
            with mock.patch.dict(os.environ, {}, clear=True), mock.patch.object(build_pgo, "run_command") as run:
                result = build_pgo.build_stage(root, out, "optimized", ["-Cprofile-use=/data with spaces/model"], compiler, 123, manifest)
            command, _, environment, _, timeout = run.call_args.args
            self.assertEqual(result, binary)
            self.assertIn("--locked", command)
            self.assertEqual(command[command.index("--target") + 1], "test-host")
            self.assertEqual(command[command.index("--target-dir") + 1], str(out / "build/optimized"))
            self.assertEqual(command[command.index("--bin") + 1], "jakgro")
            self.assertEqual(timeout, 123)
            self.assertEqual(environment["CARGO_ENCODED_RUSTFLAGS"], "-Cprofile-use=/data with spaces/model")
            self.assertEqual(manifest["binaries"]["optimized"]["sha256"], build_pgo.sha256(binary))

    def test_command_failure_points_to_retained_log(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            log = root / "build.log"
            with mock.patch.object(build_pgo.subprocess, "run", return_value=subprocess.CompletedProcess(["cargo"], 7)):
                with self.assertRaisesRegex(RuntimeError, "status 7"):
                    build_pgo.run_command(["cargo"], root, {}, log, 5)
            self.assertIn("cargo", log.read_text())


class ObservationTests(unittest.TestCase):
    def test_iterations_keep_pv_but_ignore_clock_and_throughput(self):
        lines = ["info string personality attempts 1", "info depth 2 score cp 17 nodes 150 time 3 nps 50000 pv e2e4 e7e5", "bestmove e2e4"]
        self.assertEqual(build_pgo.search_iterations(lines), [
            {"score": "cp 17", "depth": 2, "nodes": 150, "pv": ["e2e4", "e7e5"]}
        ])

    def test_only_timing_differences_are_allowed(self):
        before = observation()
        after = copy.deepcopy(before)
        after.update(elapsed_ms=8, nps=112500)
        build_pgo.validate_observations([before], [after])
        for key, value in [("bestmove", "d2d4"), ("score", "cp 19"), ("depth", 4),
                           ("nodes", 901), ("personality", {}), ("iterations", [])]:
            with self.subTest(key=key):
                changed = copy.deepcopy(after)
                changed[key] = value
                with self.assertRaisesRegex(ValueError, "changed fixed-node"):
                    build_pgo.validate_observations([before], [changed])
        with self.assertRaises(ValueError):
            build_pgo.validate_observations([], [])
        with self.assertRaises(ValueError):
            build_pgo.validate_observations([before], [])


class LifecycleTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / "src").mkdir()
        (self.root / "src/main.rs").write_text("fn main() {}\n")
        (self.root / "Cargo.toml").write_text("[package]\nname='test'\n")
        (self.root / "Cargo.lock").write_text("lock\n")
        (self.root / "training.epd").write_text(f'{START} id "training";\n')
        (self.root / "validation.epd").write_text(f'{OTHER} id "validation";\n')
        self.out = self.root / "new output"
        self.args = build_pgo.parse_arguments([
            "--output-dir", str(self.out), "--training-suite", "training.epd",
            "--validation-suite", "validation.epd", "--nodes", "1000", "--validation-nodes", "1000",
        ])
        self.environment = mock.patch.dict(os.environ, {}, clear=True)
        self.environment.start()
        self.addCleanup(self.environment.stop)
        self.compiler = {"cargo": "cargo", "host": "test-host", "llvm_profdata": "llvm-profdata"}
        self.calls = []
        self.no_raw = False
        self.change_score = False
        self.change_source = False

    def fake_build(self, root, out, stage, flags, compiler, timeout, manifest):
        self.calls.append(stage)
        path = out / ("fake-" + stage)
        path.write_bytes(stage.encode())
        manifest["binaries"][stage] = {"path": str(path), "sha256": build_pgo.sha256(path)}
        return path

    def fake_measure(self, binary, fixtures, selected_profiles, timeout, hash_mib):
        if binary.name == "fake-instrumented" and not self.no_raw:
            (self.out / "profiles/test.profraw").write_bytes(b"profile")
        row = observation()
        if binary.name == "fake-optimized":
            if self.change_score:
                row["score"] = "cp 21"
            if self.change_source:
                (self.root / "src/main.rs").write_text("fn main() { println!(\"changed\"); }\n")
        return [row]

    def fake_command(self, command, root, environment, log, timeout):
        log.write_text("merged\n")
        Path(command[command.index("-o") + 1]).write_bytes(b"merged-profile")

    def run_build(self):
        with mock.patch.object(build_pgo, "toolchain", return_value=self.compiler), \
             mock.patch.object(build_pgo, "build_stage", side_effect=self.fake_build), \
             mock.patch.object(build_pgo, "measure", side_effect=self.fake_measure), \
             mock.patch.object(build_pgo, "run_command", side_effect=self.fake_command):
            return build_pgo.build(self.args, self.root)

    def test_success_records_inputs_and_only_publishes_after_validation(self):
        output = self.run_build()
        manifest = json.loads((self.out / "manifest.json").read_text())
        self.assertEqual(self.calls, ["baseline", "instrumented", "optimized"])
        self.assertEqual(manifest["status"], "complete")
        self.assertEqual(manifest["output"]["sha256"], build_pgo.sha256(output))
        self.assertIn("src/main.rs", manifest["source_inputs"])
        self.assertEqual(manifest["profile_sha256"], build_pgo.sha256(self.out / "merged.profdata"))
        self.assertFalse((self.root / "target").exists())

    def test_existing_output_is_never_reused_or_deleted(self):
        self.out.mkdir()
        marker = self.out / "keep"
        marker.write_text("mine")
        with self.assertRaisesRegex(ValueError, "already exists"):
            self.run_build()
        self.assertEqual(marker.read_text(), "mine")
        self.assertEqual(self.calls, [])

    def test_missing_profiles_fail_without_publishing(self):
        self.no_raw = True
        with self.assertRaisesRegex(ValueError, "no usable raw"):
            self.run_build()
        self.assertEqual(json.loads((self.out / "manifest.json").read_text())["status"], "failed")
        self.assertFalse((self.out / "jakgro-pgo").exists())
        self.assertEqual(self.calls, ["baseline", "instrumented"])

    def test_changed_search_is_recorded_as_failure(self):
        self.change_score = True
        with self.assertRaisesRegex(ValueError, "changed fixed-node"):
            self.run_build()
        manifest = json.loads((self.out / "manifest.json").read_text())
        self.assertEqual(manifest["status"], "failed")
        self.assertIn("changed fixed-node", manifest["error"])
        self.assertTrue((self.out / "validation.json").exists())
        self.assertFalse((self.out / "jakgro-pgo").exists())

    def test_source_mutation_is_detected(self):
        self.change_source = True
        with self.assertRaisesRegex(ValueError, "inputs changed"):
            self.run_build()
        self.assertEqual(json.loads((self.out / "manifest.json").read_text())["status"], "failed")

    def test_training_restores_ambient_profile_override_on_failure(self):
        os.environ["LLVM_PROFILE_FILE"] = "keep-this-setting"
        self.no_raw = True
        with self.assertRaises(ValueError):
            self.run_build()
        self.assertEqual(os.environ["LLVM_PROFILE_FILE"], "keep-this-setting")


if __name__ == "__main__":
    unittest.main()
