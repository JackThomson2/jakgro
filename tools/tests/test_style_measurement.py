import argparse
import contextlib
import io
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

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


class SearchInfoParsingTests(unittest.TestCase):
    def test_optional_personality_debug_counters(self) -> None:
        self.assertEqual(measure_style.parse_personality_info("info depth 3"), {})
        self.assertEqual(
            measure_style.parse_personality_info(
                "info string personality nodes=2048 completed=2 exhausted=1 invalid=x"
            ),
            {"nodes": 2048, "completed": 2, "exhausted": 1},
        )

    def test_parser_retains_engine_timing_and_throughput(self) -> None:
        parsed = measure_style.parse_search_info(
            "info depth 6 score cp 21 nodes 12345 time 67 nps 184253 pv e2e4"
        )

        self.assertEqual(parsed, ("cp 21", 6, 12345, 67, 184253))


class FixedPositionSummaryTests(unittest.TestCase):
    def test_standard_suite_rates_all_three_profiles(self) -> None:
        fixtures = measure_style.parse_suite(Path("tests/data/standard-attacks.epd"))
        self.assertTrue(all(set(f.expected) == {0, 75, 100} for f in fixtures))

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

    def test_standard_improvement_needs_two_motifs_and_preserved_controls(self) -> None:
        def row(identifier, category, before="FAIL", after="pass", profile=75):
            return {
                "id": identifier, "category": category, "aggression": profile,
                "bestmove": "e2e4" if after == "pass" else "d2d4", "expected": "e2e4",
                "score": "cp 20", "depth": 8, "nodes": 20000, "status": after,
                "baseline_bestmove": "e2e4" if before == "pass" else "d2d4",
                "baseline_score": "cp 20", "baseline_depth": 8, "baseline_nodes": 20000,
                "baseline_status": before,
                "move_changed": before != after,
            }

        def passes(rows):
            result = measure_style.summarize_comparison(rows, self.candidate, self.baseline, self.suite)
            return result["gates"]["standard_attacks_improved"]["passed"]

        first = row("pressure", "forcing-attack")
        self.assertFalse(passes([first, row("another-pressure", "forcing-attack")]))
        both = [first, row("storm", "pawn-storm")]
        self.assertTrue(passes(both))
        self.assertFalse(passes(both + [row("lost-attack", "king-attack", "pass", "FAIL")]))
        self.assertFalse(passes(both + [row("unsafe", "safety", "pass", "FAIL")]))
        self.assertFalse(passes([first, row("wrong-profile", "pawn-storm", profile=100)]))

    def test_standard_cli_accepts_two_gains_with_an_unimproved_target(self) -> None:
        fixtures = [
            measure_style.Fixture(str(i), category, "unused", 100,
                                 {75: frozenset({"e2e4"})})
            for i, category in enumerate(("forcing-attack", "pawn-storm", "initiative"))
        ]
        candidate_path = self.candidate

        class FakeEngine:
            def __init__(self, executable, _timeout):
                self.candidate = executable == candidate_path

            def __enter__(self):
                return self

            def __exit__(self, *_args):
                pass

            def measure(self, fixture, _profile):
                move = "e2e4" if self.candidate and fixture.identifier != "2" else "d2d4"
                return measure_style.Observation(move, "cp 20", 8, 100)

        argv = ["measure_style", "--engine", str(self.candidate), "--baseline-engine",
                str(self.baseline), "--suite", str(self.suite), "--profiles", "75",
                "--require-standard-improvement"]
        with patch("sys.argv", argv), patch.object(measure_style, "parse_suite", return_value=fixtures), \
             patch.object(measure_style, "UciEngine", FakeEngine), \
             contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(measure_style.main(), 0)

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

    def test_a_control_re_pinned_away_from_the_baseline_is_judged_by_the_suite(self) -> None:
        def row(bestmove: str, status: str, baseline_bestmove: str, baseline_status: str):
            return {
                "id": "control",
                "category": "anti-sacrifice",
                "aggression": 100,
                "bestmove": bestmove,
                "expected": "b1d2",
                "score": "cp 43",
                "depth": 6,
                "nodes": 100000,
                "status": status,
                "baseline_bestmove": baseline_bestmove,
                "baseline_score": "cp 49",
                "baseline_depth": 6,
                "baseline_nodes": 100000,
                "baseline_status": baseline_status,
                "move_changed": bestmove != baseline_bestmove,
                "expected_hit_delta": int(status == "pass") - int(baseline_status == "pass"),
            }

        # The suite expects b1d2, the candidate plays it, the baseline still
        # plays the c1f4 the suite used to pin: preserved, by the suite.
        summary = measure_style.summarize_comparison(
            [row("b1d2", "pass", "c1f4", "FAIL")], self.candidate, self.baseline, self.suite
        )
        self.assertTrue(summary["gates"]["controls_preserved"]["passed"])
        # A candidate that misses the suite's move fails whatever the baseline
        # does, and one that changes a move the baseline still hits fails too.
        summary = measure_style.summarize_comparison(
            [row("d3h7", "FAIL", "c1f4", "FAIL")], self.candidate, self.baseline, self.suite
        )
        self.assertFalse(summary["gates"]["controls_preserved"]["passed"])
        summary = measure_style.summarize_comparison(
            [row("c1e3", "pass", "b1d2", "pass")], self.candidate, self.baseline, self.suite
        )
        self.assertFalse(summary["gates"]["controls_preserved"]["passed"])


class FrozenSacrificeSuiteTests(unittest.TestCase):
    def test_suite_contains_positive_and_control_positions(self) -> None:
        fixtures = measure_style.parse_suite(Path("tests/data/sacrifice-gates.epd"))

        self.assertEqual(len(fixtures), 5)
        self.assertEqual(
            {fixture.category for fixture in fixtures},
            {"sacrifice", "anti-sacrifice", "safety"},
        )
        self.assertTrue(all(100 in fixture.expected for fixture in fixtures))
        self.assertEqual(
            measure_style.sha256_file(Path("tests/data/sacrifice-gates.epd")),
            "79ac6e6b5722c251236e8ec33a2f04ece5acc2480c94f2772c6ef9c86e80907c",
        )


if __name__ == "__main__":
    unittest.main()
