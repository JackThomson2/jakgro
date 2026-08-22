import argparse
import tempfile
import unittest
from pathlib import Path

from tools import analyze_match, measure_style, run_match


class MatchIdentityTests(unittest.TestCase):
    def test_same_aggression_profiles_receive_distinct_default_names(self) -> None:
        args = argparse.Namespace(
            candidate_aggression=100,
            baseline_aggression=100,
            candidate_name=None,
            baseline_name=None,
        )

        self.assertEqual(
            run_match.engine_names(args),
            ("Candidate-Aggression-100", "Baseline-Aggression-100"),
        )

    def test_explicit_engine_names_must_differ(self) -> None:
        args = argparse.Namespace(
            candidate_aggression=100,
            baseline_aggression=100,
            candidate_name="Current",
            baseline_name="Current",
        )

        with self.assertRaisesRegex(ValueError, "must differ"):
            run_match.engine_names(args)


class MovetextStyleTests(unittest.TestCase):
    def test_parser_retains_annotated_mainline_moves(self) -> None:
        pgn = '''[Event "style"]
[White "Candidate"]
[Black "Baseline"]
[Result "1-0"]
[PlyCount "5"]

1. e4 {book} e5 (1... c5) 2. Qh5!? Nc6 3. Qxf7+ 1-0
'''
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "match.pgn"
            path.write_text(pgn, encoding="utf-8")
            games = analyze_match.parse_pgn(path)

        self.assertEqual(games[0].moves, ("e4", "e5", "Qh5", "Nc6", "Qxf7+"))
        indicators = analyze_match.style_indicators(games, "Candidate", "Baseline")
        self.assertEqual(indicators["candidate"]["moves"], 3)
        self.assertEqual(indicators["candidate"]["checks"], 1)
        self.assertEqual(indicators["candidate"]["captures"], 1)
        self.assertEqual(indicators["candidate"]["forcing_moves_per_100_moves"], 33.333333)

    def test_fen_active_color_assigns_the_first_move(self) -> None:
        game = analyze_match.Game(
            event="style",
            white="Baseline",
            black="Candidate",
            result="1/2-1/2",
            termination="normal",
            fen="7k/8/8/8/8/8/8/K7 b - - 0 1",
            ply_count=2,
            moves=("Qh4+", "g3"),
        )

        indicators = analyze_match.style_indicators([game], "Candidate", "Baseline")

        self.assertEqual(indicators["candidate"]["checks"], 1)
        self.assertEqual(indicators["baseline"]["checks"], 0)


class FixedPositionSummaryTests(unittest.TestCase):
    def test_suite_accepts_categories_and_multiple_expected_moves(self) -> None:
        suite = (
            "7k/8/8/8/8/8/8/K7 w - - 0 1 ; id attack ; category king-attack ; "
            "nodes 100 ; bm0 a1a2 ; bm100 a1a2,a1b1\n"
        )
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "style.epd"
            path.write_text(suite, encoding="utf-8")
            fixtures = measure_style.parse_suite(path)

        self.assertEqual(fixtures[0].category, "king-attack")
        self.assertEqual(fixtures[0].expected[100], frozenset({"a1a2", "a1b1"}))

    def test_summary_groups_hits_by_category_and_profile(self) -> None:
        rows = [
            {"category": "attack", "aggression": 100, "expected": "a1a2", "status": "pass"},
            {"category": "attack", "aggression": 100, "expected": "a1b1", "status": "FAIL"},
            {"category": "attack", "aggression": 50, "expected": "", "status": "unrated"},
        ]

        summary = measure_style.summarize(rows)

        profile = summary["categories"]["attack"]["100"]
        self.assertEqual(profile["rated"], 2)
        self.assertEqual(profile["hits"], 1)
        self.assertEqual(profile["hit_rate_percent"], 50.0)


class BinaryComparisonSummaryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        root = Path(self.temporary.name)
        self.candidate = root / "candidate"
        self.baseline = root / "baseline"
        self.suite = root / "suite.epd"
        self.candidate.write_bytes(b"candidate")
        self.baseline.write_bytes(b"baseline")
        self.suite.write_text("suite\n", encoding="utf-8")

    def test_comparison_records_hashes_and_category_deltas(self) -> None:
        rows = [
            {
                "id": "sound-sacrifice",
                "category": "sacrifice",
                "aggression": 100,
                "bestmove": "c4f7",
                "expected": "c4f7",
                "score": "cp 25",
                "depth": 8,
                "nodes": 20000,
                "status": "pass",
                "baseline_bestmove": "g5f7",
                "baseline_score": "cp 31",
                "baseline_depth": 8,
                "baseline_nodes": 20000,
                "baseline_status": "FAIL",
                "move_changed": True,
                "expected_hit_delta": 1,
            },
            {
                "id": "unsound-sacrifice",
                "category": "anti-sacrifice",
                "aggression": 100,
                "bestmove": "f1e1",
                "expected": "f1e1",
                "score": "cp 4",
                "depth": 8,
                "nodes": 20000,
                "status": "pass",
                "baseline_bestmove": "f1e1",
                "baseline_score": "cp 4",
                "baseline_depth": 8,
                "baseline_nodes": 20000,
                "baseline_status": "pass",
                "move_changed": False,
                "expected_hit_delta": 0,
            },
        ]

        summary = measure_style.summarize_comparison(
            rows, self.candidate, self.baseline, self.suite
        )

        sacrifice = summary["categories"]["sacrifice"]["100"]
        self.assertEqual(sacrifice["hit_delta"], 1)
        self.assertEqual(sacrifice["improvements"], 1)
        self.assertTrue(summary["distinct_binaries"])
        self.assertTrue(summary["gates"]["candidate_expected_moves"]["passed"])
        self.assertTrue(summary["gates"]["controls_preserved"]["passed"])
        self.assertTrue(summary["gates"]["sacrifice_improved"]["passed"])
        self.assertEqual(
            summary["inputs"]["candidate"]["sha256"],
            measure_style.sha256_file(self.candidate),
        )

    def test_changed_control_move_fails_the_preservation_gate(self) -> None:
        rows = [
            {
                "id": "control",
                "category": "safety",
                "aggression": 100,
                "bestmove": "a1b1",
                "expected": "a1a2",
                "score": "cp -600",
                "depth": 8,
                "nodes": 20000,
                "status": "FAIL",
                "baseline_bestmove": "a1a2",
                "baseline_score": "cp -590",
                "baseline_depth": 8,
                "baseline_nodes": 20000,
                "baseline_status": "pass",
                "move_changed": True,
                "expected_hit_delta": -1,
            }
        ]

        summary = measure_style.summarize_comparison(
            rows, self.candidate, self.baseline, self.suite
        )

        gate = summary["gates"]["controls_preserved"]
        self.assertFalse(gate["passed"])
        self.assertEqual(gate["failed_positions"], ["control@100"])


class FrozenSacrificeSuiteTests(unittest.TestCase):
    def test_suite_contains_positive_and_control_positions(self) -> None:
        fixtures = measure_style.parse_suite(Path("tests/data/sacrifice-gates.epd"))

        self.assertEqual(len(fixtures), 4)
        self.assertEqual(
            {fixture.category for fixture in fixtures},
            {"sacrifice", "anti-sacrifice", "safety"},
        )
        self.assertTrue(all(100 in fixture.expected for fixture in fixtures))
        self.assertEqual(
            measure_style.sha256_file(Path("tests/data/sacrifice-gates.epd")),
            "2abb0a941162ed5157b44ac5e6a9c93f969b2a55d1c271c1e8a3a86e44622fbe",
        )


if __name__ == "__main__":
    unittest.main()
