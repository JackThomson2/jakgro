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
            blocks.append(
                f'[Event "game {index}"]\n'
                f'[White "{white}"]\n'
                f'[Black "{black}"]\n'
                f'[Result "{result}"]\n'
                f'[Termination "{termination}"]\n'
                f'[PlyCount "{plies}"]\n'
                f'[FEN "{fen}"]\n\n'
                f'1. e4 {result}\n'
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
            self.assertEqual(summary["confidence"]["score_percent_ci95"], [38.0, 87.0])
            self.assertAlmostEqual(summary["confidence"]["elo"], 88.7395, places=3)
            self.assertEqual(summary["average_plies"], 55.0)
            self.assertIn("W/D/L: 2/1/1", analyze_match.markdown(summary))

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


if __name__ == "__main__":
    unittest.main()
