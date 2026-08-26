import argparse
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import unittest
from datetime import datetime, timedelta, timezone
from pathlib import Path
from unittest.mock import patch

from tools import run_match


class RunMatchTests(unittest.TestCase):
    def test_repository_opening_corpus_has_48_unique_pairs(self) -> None:
        self.assertEqual(run_match.count_openings(Path("tools/data/openings.epd")), 48)

    def test_duplicate_openings_and_ids_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "openings.epd"
            path.write_text(
                "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - id \"one\";\n"
                "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - id \"two\";\n",
                encoding="utf-8",
            )
            with self.assertRaisesRegex(ValueError, "duplicate opening position"):
                run_match.count_openings(path)

    def test_manifest_records_immutable_inputs_and_settings(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            engine = root / "engine"
            cutechess = root / "cutechess-cli"
            openings = root / "openings.epd"
            pgn = root / "match.pgn"
            manifest_path = root / "match.manifest.json"
            engine.write_bytes(b"candidate")
            cutechess.write_bytes(b"cutechess")
            openings.write_text(
                "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - id \"start\";\n",
                encoding="utf-8",
            )
            args = argparse.Namespace(
                engine=engine,
                baseline_engine=engine,
                candidate_aggression=100,
                baseline_aggression=0,
                games=2,
                nodes=50_000,
                time_control=None,
                hash=16,
                openings=openings,
                pgn=pgn,
                manifest=manifest_path,
                cutechess=cutechess,
                candidate_revision="candidate-rev",
                baseline_revision="baseline-rev",
                dependency_revision="cozy-rev",
                build_profile="release",
            )
            command = run_match.build_command(args)
            self.assertIn("tc=inf", command)
            self.assertIn("nodes=50000", command)

            manifest = run_match.build_manifest(args, command, 1, "cutechess-cli 1.3.1")
            pgn.write_text(
                '[Event "game one"]\n[Result "1-0"]\n\n1. e4 1-0\n\n'
                '[Event "game two"]\n[Result "0-1"]\n\n1. d4 0-1\n',
                encoding="utf-8",
            )
            started = datetime(2026, 1, 1, tzinfo=timezone.utc)
            completed, complete = run_match.record_execution(
                manifest,
                args,
                started,
                started + timedelta(seconds=5),
                0,
                None,
            )
            self.assertEqual(completed, 2)
            self.assertTrue(complete)
            run_match.write_manifest(manifest_path, manifest)
            persisted = json.loads(manifest_path.read_text(encoding="utf-8"))

            self.assertEqual(persisted["schema_version"], 2)
            self.assertEqual(persisted["command"], command)
            self.assertEqual(persisted["settings"]["nodes_per_move"], 50_000)
            self.assertEqual(persisted["settings"]["limit"]["mode"], "fixed-nodes")
            self.assertIsNone(persisted["settings"]["time_control"])
            self.assertEqual(persisted["provenance"]["dependency_revision"], "cozy-rev")
            self.assertEqual(persisted["inputs"]["candidate"]["revision"], "candidate-rev")
            self.assertEqual(persisted["settings"]["concurrency"], 1)
            self.assertEqual(
                persisted["inputs"]["candidate"]["sha256"],
                hashlib.sha256(b"candidate").hexdigest(),
            )
            self.assertEqual(persisted["inputs"]["openings"]["count"], 1)
            self.assertIn("runner", persisted["inputs"])
            self.assertEqual(persisted["execution"]["status"], "complete")
            self.assertEqual(persisted["execution"]["completed_games"], 2)
            self.assertEqual(persisted["execution"]["duration_seconds"], 5.0)
            self.assertEqual(
                persisted["execution"]["pgn_sha256"],
                run_match.sha256_file(pgn),
            )
            self.assertFalse(manifest_path.with_suffix(".json.tmp").exists())

    def test_pgn_counter_counts_games(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "match.pgn"
            path.write_text(
                '[Event "game one"]\n[Result "1-0"]\n\n1. e4 1-0\n\n'
                '[Event "game two"]\n[Result "1/2-1/2"]\n\n1. d4 1/2-1/2\n',
                encoding="utf-8",
            )
            self.assertEqual(run_match.count_pgn_games(path), 2)
            path.write_text('[Event "unfinished"]\n[Result "1-0"]\n', encoding="utf-8")
            self.assertEqual(run_match.count_pgn_games(path), 0)

    def test_cli_writes_completed_manifest_with_fake_cutechess(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            engine = root / "engine"
            cutechess = root / "cutechess-cli"
            openings = root / "openings.epd"
            pgn = root / "match.pgn"
            manifest = root / "match.manifest.json"
            engine.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
            cutechess.write_text(
                "#!/usr/bin/env python3\n"
                "import pathlib, sys\n"
                "if '--version' in sys.argv:\n"
                "    print('cutechess-cli fake-1')\n"
                "    raise SystemExit(0)\n"
                "pgn = pathlib.Path(sys.argv[sys.argv.index('-pgnout') + 1])\n"
                "rounds = int(sys.argv[sys.argv.index('-rounds') + 1])\n"
                "games = int(sys.argv[sys.argv.index('-games') + 1])\n"
                "pgn.write_text(''.join(f'[Event \\\"game {i}\\\"]\\n[Result \\\"1-0\\\"]\\n\\n1. e4 1-0\\n\\n' for i in range(rounds * games)))\n",
                encoding="utf-8",
            )
            os.chmod(engine, 0o755)
            os.chmod(cutechess, 0o755)
            openings.write_text(
                "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - id \"start\";\n",
                encoding="utf-8",
            )

            result = subprocess.run(
                [
                    sys.executable,
                    str(Path("tools/run_match.py").resolve()),
                    "--engine",
                    str(engine),
                    "--cutechess",
                    str(cutechess),
                    "--openings",
                    str(openings),
                    "--games",
                    "2",
                    "--nodes",
                    "1",
                    "--pgn",
                    str(pgn),
                    "--manifest",
                    str(manifest),
                ],
                check=False,
                capture_output=True,
                text=True,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            persisted = json.loads(manifest.read_text(encoding="utf-8"))
            self.assertEqual(persisted["execution"]["status"], "complete")
            self.assertEqual(persisted["execution"]["completed_games"], 2)
            self.assertEqual(run_match.count_pgn_games(pgn), 2)

    def test_executable_resolution_and_version_query(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            executable = Path(directory) / "cutechess-cli"
            executable.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
            os.chmod(executable, 0o755)
            self.assertEqual(run_match.resolve_executable(executable), executable.resolve())

            completed = subprocess.CompletedProcess(
                [str(executable), "--version"],
                0,
                stdout="cutechess-cli 1.3.1\n",
                stderr="",
            )
            with patch.object(run_match.subprocess, "run", return_value=completed):
                self.assertEqual(
                    run_match.read_cutechess_version(executable),
                    "cutechess-cli 1.3.1",
                )

    def test_fixed_time_command_omits_node_limit(self) -> None:
        args = argparse.Namespace(
            engine=Path("candidate"),
            baseline_engine=Path("baseline"),
            candidate_aggression=75,
            baseline_aggression=75,
            games=96,
            nodes=None,
            time_control="10+0.1",
            hash=16,
            openings=Path("openings.epd"),
            pgn=Path("match.pgn"),
            cutechess=Path("cutechess-cli"),
        )

        command = run_match.build_command(args)

        self.assertIn("tc=10+0.1", command)
        self.assertNotIn("tc=inf", command)
        self.assertFalse(any(argument.startswith("nodes=") for argument in command))

    def test_time_control_parser_accepts_cute_chess_forms(self) -> None:
        for value in ("10+0.1", "40/60+0.5", "1:00+0.5", "0.5"):
            with self.subTest(value=value):
                self.assertEqual(run_match.time_control(value), value)
        for value in ("inf", "10 seconds", "40//60", "10+", ""):
            with self.subTest(value=value):
                with self.assertRaises(argparse.ArgumentTypeError):
                    run_match.time_control(value)


class BinaryComparisonValidationTests(unittest.TestCase):
    def test_same_profile_requires_distinct_binary_hashes(self) -> None:
        with self.assertRaisesRegex(ValueError, "different hashes"):
            run_match.validate_binary_comparison("same", "same", 100, 100)

    def test_profile_self_play_may_reuse_one_binary(self) -> None:
        run_match.validate_binary_comparison("same", "same", 100, 0)

    def test_same_profile_accepts_distinct_binary_hashes(self) -> None:
        run_match.validate_binary_comparison("candidate", "baseline", 100, 100)


if __name__ == "__main__":
    unittest.main()
