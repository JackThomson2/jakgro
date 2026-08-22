import json
import subprocess
import sys
import tempfile
import unittest
from dataclasses import replace
from pathlib import Path

from tools import analyze_match


class AnalyzeMatchTests(unittest.TestCase):
    def write_match(self, root: Path) -> tuple[Path, Path]:
        pgn = root / "match.pgn"
        manifest = root / "match.manifest.json"
        first_fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"
        second_fen = "rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2"
        games = [
            ("Aggression-100", "Aggression-0", "1-0", "adjudication", 40, first_fen),
            ("Aggression-0", "Aggression-100", "1/2-1/2", "normal", 60, first_fen),
            ("Aggression-100", "Aggression-0", "0-1", "adjudication", 50, second_fen),
            ("Aggression-0", "Aggression-100", "0-1", "checkmate", 70, second_fen),
        ]
        blocks = []
        for index, (white, black, result, termination, plies, fen) in enumerate(games, 1):
            movetext = " ".join(
                f"{move}. e4 e5" for move in range(1, plies // 2 + 1)
            )
            blocks.append(
                f'[Event "game {index}"]\n'
                f'[White "{white}"]\n'
                f'[Black "{black}"]\n'
                f'[Result "{result}"]\n'
                f'[Termination "{termination}"]\n'
                f'[PlyCount "{plies}"]\n'
                f'[FEN "{fen}"]\n\n'
                f'{movetext} {result}\n'
            )
        pgn.write_text("\n".join(blocks), encoding="utf-8")
        manifest.write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "inputs": {
                        "candidate": {"aggression": 100},
                        "baseline": {"aggression": 0},
                    },
                    "settings": {"games": 4},
                    "execution": {
                        "status": "complete",
                        "completed_games": 4,
                        "pgn_sha256": analyze_match.sha256_file(pgn),
                    },
                }
            ),
            encoding="utf-8",
        )
        return pgn, manifest

    def test_summary_reports_wdl_colors_pairs_and_confidence(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            pgn, manifest_path = self.write_match(Path(directory))
            manifest, candidate, baseline, expected = analyze_match.load_manifest(
                manifest_path, pgn
            )
            games = analyze_match.parse_pgn(pgn)
            summary = analyze_match.summarize(
                games,
                manifest,
                candidate,
                baseline,
                pgn,
                manifest_path,
            )

            self.assertEqual(expected, 4)
            self.assertEqual(summary["result"]["wins"], 2)
            self.assertEqual(summary["result"]["draws"], 1)
            self.assertEqual(summary["result"]["losses"], 1)
            self.assertEqual(summary["result"]["score_percent"], 62.5)
            self.assertEqual(summary["result"]["decisive_percent"], 75.0)
            self.assertEqual(summary["colors"]["white"]["score_percent"], 50.0)
            self.assertEqual(summary["colors"]["black"]["score_percent"], 75.0)
            self.assertEqual(summary["pairs"]["point_distribution"], {"1.0": 1, "1.5": 1})
            self.assertEqual(summary["pairs"]["decisive_splits"], 1)
            self.assertEqual(summary["confidence"]["score_percent_ci95"], [0.0, 100.0])
            self.assertAlmostEqual(summary["confidence"]["elo"], 88.7395, places=3)
            self.assertEqual(summary["average_plies"], 55.0)
            self.assertIn("W/D/L: 2/1/1", analyze_match.markdown(summary))
            rendered = analyze_match.markdown(summary)
            self.assertIn(
                "Confidence method: 95% Hoeffding bound over color-reversed pair scores.",
                rendered,
            )
            self.assertNotIn("normal interval", rendered)

    def test_manifest_hash_and_pairing_are_enforced(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            pgn, manifest_path = self.write_match(Path(directory))
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
            manifest["execution"]["pgn_sha256"] = "0" * 64
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "PGN hash"):
                analyze_match.load_manifest(manifest_path, pgn)

            pgn, manifest_path = self.write_match(Path(directory))
            manifest, candidate, baseline, _ = analyze_match.load_manifest(manifest_path, pgn)
            games = analyze_match.parse_pgn(pgn)
            games[1] = replace(
                games[1],
                white="Aggression-100",
                black="Aggression-0",
            )
            with self.assertRaisesRegex(ValueError, "not color-reversed"):
                analyze_match.summarize(
                    games,
                    manifest,
                    candidate,
                    baseline,
                    pgn,
                    manifest_path,
                )

    def test_incomplete_results_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            pgn = Path(directory) / "match.pgn"
            pgn.write_text(
                '[Event "unfinished"]\n[White "A"]\n[Black "B"]\n[Result "*"]\n',
                encoding="utf-8",
            )
            with self.assertRaisesRegex(ValueError, "incomplete result"):
                analyze_match.parse_pgn(pgn)

    def test_cli_writes_json_and_markdown(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pgn, manifest = self.write_match(root)
            summary_json = root / "summary.json"
            summary_markdown = root / "summary.md"
            result = subprocess.run(
                [
                    sys.executable,
                    str(Path("tools/analyze_match.py").resolve()),
                    "--pgn",
                    str(pgn),
                    "--manifest",
                    str(manifest),
                    "--json",
                    str(summary_json),
                    "--markdown",
                    str(summary_markdown),
                ],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                json.loads(summary_json.read_text(encoding="utf-8"))["games"],
                4,
            )
            self.assertIn(
                "Aggression paired-match summary",
                summary_markdown.read_text(encoding="utf-8"),
            )


class EloLowerBoundCliTests(unittest.TestCase):
    def test_failing_gate_still_writes_auditable_outputs(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pgn, manifest = AnalyzeMatchTests().write_match(root)
            summary_json = root / "summary.json"
            summary_markdown = root / "summary.md"
            result = subprocess.run(
                [
                    sys.executable,
                    str(Path("tools/analyze_match.py").resolve()),
                    "--pgn",
                    str(pgn),
                    "--manifest",
                    str(manifest),
                    "--json",
                    str(summary_json),
                    "--markdown",
                    str(summary_markdown),
                    "--min-elo-lower-bound",
                    "0",
                ],
                check=False,
                capture_output=True,
                text=True,
            )

            self.assertEqual(result.returncode, 1)
            summary = json.loads(summary_json.read_text(encoding="utf-8"))
            self.assertFalse(summary["gates"]["elo_lower_bound"]["passed"])
            self.assertIn(
                "Elo lower bound: **FAIL**",
                summary_markdown.read_text(encoding="utf-8"),
            )
            self.assertIn("Elo lower-bound gate failed", result.stderr)


class EloLowerBoundGateTests(unittest.TestCase):
    def test_gate_requires_the_interval_to_clear_the_threshold(self) -> None:
        summary = {
            "confidence": {
                "score_percent_ci95": [51.0, 57.0],
                "elo_ci95": [7.0, 49.0],
            }
        }

        gate = analyze_match.elo_lower_bound_gate(summary, 0.0)

        self.assertTrue(gate["passed"])
        self.assertEqual(gate["required_score_percent"], 50.0)
        self.assertEqual(gate["observed_elo_lower"], 7.0)

    def test_gate_is_strict_and_handles_nonzero_elo_thresholds(self) -> None:
        threshold = 20.0
        required = analyze_match.percentage(analyze_match.score_from_elo(threshold))
        summary = {
            "confidence": {
                "score_percent_ci95": [required, 60.0],
                "elo_ci95": [threshold, 70.0],
            }
        }

        gate = analyze_match.elo_lower_bound_gate(summary, threshold)

        self.assertFalse(gate["passed"])


if __name__ == "__main__":
    unittest.main()
