import argparse
import json
import math
import tempfile
import unittest
from datetime import datetime, timezone
from pathlib import Path

from tools import run_sprt


class PairStatisticsTests(unittest.TestCase):
    def test_pair_distribution_buckets_every_half_point_outcome(self) -> None:
        counts = run_sprt.pair_distribution([0.0, 0.5, 1.0, 1.0, 1.5, 2.0])

        self.assertEqual(counts, [1, 1, 2, 1, 1])

    def test_pair_distribution_rejects_impossible_scores(self) -> None:
        with self.assertRaisesRegex(ValueError, "not a multiple of half a point"):
            run_sprt.pair_distribution([0.75])

    def test_pair_statistics_reports_a_per_game_mean_and_paired_variance(self) -> None:
        statistics = run_sprt.pair_statistics([2.0, 0.0, 1.0, 1.0])

        self.assertEqual(statistics["pairs"], 4)
        self.assertEqual(statistics["mean"], 0.5)
        self.assertAlmostEqual(statistics["variance"], 1.0 / 6.0)
        self.assertAlmostEqual(statistics["sigma"], math.sqrt(1.0 / 24.0))

    def test_a_single_pair_has_no_variance_and_no_interval_width(self) -> None:
        statistics = run_sprt.pair_statistics([2.0])

        self.assertEqual(statistics["variance"], 0.0)
        self.assertEqual(run_sprt.normal_interval(statistics), (1.0, 1.0))

    def test_pair_statistics_requires_a_pair(self) -> None:
        with self.assertRaisesRegex(ValueError, "at least one pair"):
            run_sprt.pair_statistics([])

    def test_all_draws_place_the_interval_around_fifty_percent(self) -> None:
        statistics = run_sprt.pair_statistics([1.0] * 40)
        low, high = run_sprt.normal_interval(statistics)

        self.assertEqual(statistics["mean"], 0.5)
        self.assertEqual((low, high), (0.5, 0.5))


class EloConversionTests(unittest.TestCase):
    def test_score_and_elo_are_inverses(self) -> None:
        for elo in (-400.0, -25.0, 0.0, 10.0, 300.0):
            self.assertAlmostEqual(
                run_sprt.elo_from_score(run_sprt.score_from_elo(elo)), elo
            )

    def test_an_even_score_is_zero_elo_and_sweeps_are_unbounded(self) -> None:
        self.assertEqual(run_sprt.score_from_elo(0.0), 0.5)
        self.assertEqual(run_sprt.elo_from_score(0.5), 0.0)
        self.assertIsNone(run_sprt.elo_from_score(0.0))
        self.assertIsNone(run_sprt.elo_from_score(1.0))


class SequentialTestTests(unittest.TestCase):
    def test_boundaries_follow_walds_bounds(self) -> None:
        decision = run_sprt.sprt_decision(0.0, 0.05, 0.05)

        self.assertAlmostEqual(decision["lower_bound"], math.log(0.05 / 0.95), places=6)
        self.assertAlmostEqual(decision["upper_bound"], math.log(0.95 / 0.05), places=6)
        self.assertEqual(decision["decision"], "continue")

    def test_crossing_a_boundary_accepts_the_matching_hypothesis(self) -> None:
        upper = math.log(0.95 / 0.05)

        self.assertEqual(
            run_sprt.sprt_decision(upper, 0.05, 0.05)["decision"], "accept_h1"
        )
        self.assertEqual(
            run_sprt.sprt_decision(-upper, 0.05, 0.05)["decision"], "accept_h0"
        )

    def test_a_wider_indifference_region_needs_less_evidence(self) -> None:
        strong = [2.0, 2.0, 1.0, 2.0, 1.0, 2.0] * 20

        narrow = run_sprt.log_likelihood_ratio(strong, 0.0, 5.0)
        wide = run_sprt.log_likelihood_ratio(strong, 0.0, 40.0)

        self.assertGreater(wide, narrow)
        self.assertGreater(narrow, 0.0)

    def test_a_losing_match_drives_the_ratio_negative(self) -> None:
        losing = [0.0, 0.0, 1.0, 0.0, 1.0, 0.0] * 20

        self.assertLess(run_sprt.log_likelihood_ratio(losing, 0.0, 10.0), 0.0)

    def test_an_undecided_match_stays_undecided(self) -> None:
        self.assertEqual(run_sprt.log_likelihood_ratio([1.0] * 50, 0.0, 10.0), 0.0)
        self.assertEqual(run_sprt.log_likelihood_ratio([2.0, 0.0], 5.0, 5.0), 0.0)

    def test_a_strong_result_eventually_accepts_the_alternative(self) -> None:
        pairs = [2.0, 1.0] * 300

        evaluation = run_sprt.evaluate(pairs, 0.0, 10.0, 0.05, 0.05)

        self.assertEqual(evaluation["games"], 1200)
        self.assertGreater(evaluation["score_percent"], 50.0)
        self.assertEqual(evaluation["sprt"]["decision"], "accept_h1")
        self.assertGreater(evaluation["los_percent"], 99.0)

    def test_evaluation_reports_bounds_and_a_pair_distribution(self) -> None:
        evaluation = run_sprt.evaluate([2.0, 0.0, 1.0, 1.5], 0.0, 10.0, 0.05, 0.05)

        self.assertEqual(evaluation["pairs"], 4)
        self.assertEqual(
            evaluation["pair_distribution"],
            {"0.0": 1, "0.5": 0, "1.0": 1, "1.5": 1, "2.0": 1},
        )
        low, high = evaluation["score_percent_ci95"]
        self.assertLessEqual(low, evaluation["score_percent"])
        self.assertLessEqual(evaluation["score_percent"], high)

    def test_likelihood_of_superiority_is_symmetric_around_an_even_score(self) -> None:
        even = run_sprt.pair_statistics([2.0, 0.0, 2.0, 0.0])

        self.assertAlmostEqual(run_sprt.los(even), 0.5)


class PgnAccountingTests(unittest.TestCase):
    def _write_pair(self, path: Path, results: tuple[str, str]) -> None:
        fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"
        games = []
        for index, result in enumerate(results):
            white, black = ("A", "B") if index == 0 else ("B", "A")
            games.append(
                "\n".join(
                    [
                        '[Event "test"]',
                        '[Round "1"]',
                        f'[White "{white}"]',
                        f'[Black "{black}"]',
                        f'[Result "{result}"]',
                        f'[FEN "{fen}"]',
                        '[PlyCount "2"]',
                        "",
                        f"1. e4 e5 {result}",
                        "",
                    ]
                )
            )
        path.write_text("\n".join(games), encoding="utf-8")

    def test_pair_points_are_read_back_from_a_played_pgn(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "match.pgn"
            self._write_pair(path, ("1-0", "0-1"))

            self.assertEqual(run_sprt.pair_points_from_pgn(path, "A", "B"), [2.0])
            self.assertEqual(run_sprt.pair_points_from_pgn(path, "B", "A"), [0.0])

    def test_a_drawn_pair_scores_one_point(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "match.pgn"
            self._write_pair(path, ("1/2-1/2", "1/2-1/2"))

            self.assertEqual(run_sprt.pair_points_from_pgn(path, "A", "B"), [1.0])

    def test_unpaired_colors_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "match.pgn"
            fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"
            path.write_text(
                "\n".join(
                    [
                        '[Event "test"]',
                        '[White "A"]',
                        '[Black "B"]',
                        '[Result "1-0"]',
                        f'[FEN "{fen}"]',
                        "",
                        "1. e4 1-0",
                        "",
                        '[Event "test"]',
                        '[White "A"]',
                        '[Black "B"]',
                        '[Result "1-0"]',
                        f'[FEN "{fen}"]',
                        "",
                        "1. e4 1-0",
                        "",
                    ]
                ),
                encoding="utf-8",
            )

            with self.assertRaisesRegex(ValueError, "not color-reversed"):
                run_sprt.pair_points_from_pgn(path, "A", "B")


class CommandTests(unittest.TestCase):
    def _arguments(self, *extra: str) -> list[str]:
        return [
            "--engine",
            "candidate",
            "--baseline-engine",
            "baseline",
            "--games",
            "4",
            *extra,
        ]

    def test_a_fixed_node_limit_is_the_default(self) -> None:
        args = run_sprt.parse_arguments(self._arguments())

        self.assertEqual(args.nodes, 50_000)
        self.assertIsNone(args.time_control)
        command = run_sprt.build_command(args, "A", "B")
        self.assertIn("--nodes", command)
        self.assertNotIn("--time-control", command)

    def test_a_time_control_replaces_the_node_limit(self) -> None:
        args = run_sprt.parse_arguments(self._arguments("--time-control", "0.25+0.002"))

        self.assertIsNone(args.nodes)
        command = run_sprt.build_command(args, "A", "B")
        self.assertIn("--time-control", command)
        self.assertNotIn("--nodes", command)

    def test_limits_are_mutually_exclusive_and_games_must_be_paired(self) -> None:
        with self.assertRaises(SystemExit):
            run_sprt.parse_arguments(
                self._arguments("--nodes", "1000", "--time-control", "1+0")
            )
        with self.assertRaises(SystemExit):
            run_sprt.parse_arguments(
                ["--engine", "candidate", "--games", "3"]
            )

    def test_the_indifference_region_must_be_ordered(self) -> None:
        with self.assertRaises(SystemExit):
            run_sprt.parse_arguments(self._arguments("--elo0", "10", "--elo1", "0"))


class ManifestTests(unittest.TestCase):
    def _namespace(self, directory: Path, **overrides: object):
        engine = directory / "candidate"
        baseline = directory / "baseline"
        openings = directory / "openings.epd"
        runner = directory / "selfplay"
        engine.write_bytes(b"candidate")
        baseline.write_bytes(b"baseline")
        runner.write_bytes(b"runner")
        openings.write_text(
            'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - id "start";\n',
            encoding="utf-8",
        )
        args = run_sprt.parse_arguments(
            [
                "--engine",
                str(engine),
                "--baseline-engine",
                str(baseline),
                "--runner",
                str(runner),
                "--openings",
                str(openings),
                "--games",
                "2",
                "--nodes",
                "1000",
            ]
        )
        for key, value in overrides.items():
            setattr(args, key, value)
        return args

    def test_the_manifest_binds_every_input_by_hash(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            args = self._namespace(root, pgn=root / "m.pgn", manifest=root / "m.json")

            manifest = run_sprt.build_manifest(args, ["cmd"], "A", "B", 1)

            self.assertEqual(manifest["schema_version"], 2)
            for name in ("runner", "harness", "candidate", "baseline", "openings"):
                digest = manifest["inputs"][name]["sha256"]
                self.assertRegex(digest, r"^[0-9a-f]{64}$")
            self.assertEqual(manifest["settings"]["limit"]["mode"], "fixed-nodes")
            self.assertEqual(manifest["settings"]["sprt"]["elo1"], 10.0)

    def test_same_profile_runs_reject_one_binary_unless_allowed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            args = self._namespace(root, pgn=root / "m.pgn", manifest=root / "m.json")
            args.baseline_engine = args.engine
            args.candidate_aggression = 75
            args.baseline_aggression = 75

            with self.assertRaisesRegex(ValueError, "must differ"):
                run_sprt.build_manifest(args, ["cmd"], "A", "B", 1)

            args.allow_identical_binaries = True
            manifest = run_sprt.build_manifest(args, ["cmd"], "A", "B", 1)
            self.assertTrue(manifest["comparison"]["identical_binaries_allowed"])
            self.assertFalse(manifest["comparison"]["distinct_binaries_required"])

    def test_derived_engine_names_are_disambiguated(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            args = self._namespace(Path(directory))
            args.candidate_aggression = 75
            args.baseline_aggression = 75

            self.assertEqual(
                run_sprt.engine_names(args),
                ("Candidate-Aggression-75", "Baseline-Aggression-75"),
            )

            args.candidate_name = "same"
            args.baseline_name = "same"
            with self.assertRaisesRegex(ValueError, "must differ"):
                run_sprt.engine_names(args)

    def test_summaries_are_written_atomically_and_sorted(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "nested" / "summary.json"

            run_sprt.write_json(path, {"b": 2, "a": 1})

            self.assertEqual(json.loads(path.read_text(encoding="utf-8")), {"a": 1, "b": 2})
            self.assertEqual(list(path.parent.glob("*.tmp")), [])


class ExecutionRecordTests(unittest.TestCase):
    def _arguments(self, root: Path, games: int = 2) -> argparse.Namespace:
        pgn = root / "match.pgn"
        fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"
        pgn.write_text(
            "\n".join(
                "\n".join(
                    [
                        '[Event "test"]',
                        f'[White "{white}"]',
                        f'[Black "{black}"]',
                        '[Result "1-0"]',
                        f'[FEN "{fen}"]',
                        "",
                        "1. e4 1-0",
                        "",
                    ]
                )
                for white, black in (("A", "B"), ("B", "A"))
            ),
            encoding="utf-8",
        )
        args = argparse.Namespace(
            pgn=pgn,
            games=games,
            results_json=root / "arbiter.json",
        )
        return args

    def test_a_clean_run_is_recorded_as_complete(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            args = self._arguments(root)
            args.results_json.write_text('{"faults": []}', encoding="utf-8")
            manifest: dict[str, object] = {}
            now = datetime.now(timezone.utc)

            completed = run_sprt.record_execution(manifest, args, now, now, 0, None)

            self.assertEqual(completed, 2)
            self.assertEqual(manifest["execution"]["status"], "complete")
            self.assertEqual(manifest["execution"]["faults"], [])

    def test_a_recorded_fault_refuses_to_report_a_complete_match(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            args = self._arguments(root)
            args.results_json.write_text(
                '{"faults": [{"engine": "A", "kind": "illegal move", "detail": "a1a1"}]}',
                encoding="utf-8",
            )
            manifest: dict[str, object] = {}
            now = datetime.now(timezone.utc)

            run_sprt.record_execution(manifest, args, now, now, 0, None)

            self.assertEqual(manifest["execution"]["status"], "failed")
            self.assertIn("fault", str(manifest["execution"]["error"]))
            self.assertEqual(len(manifest["execution"]["faults"]), 1)

    def test_a_short_match_is_recorded_as_failed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            args = self._arguments(root, games=4)
            args.results_json.write_text('{"faults": []}', encoding="utf-8")
            manifest: dict[str, object] = {}
            now = datetime.now(timezone.utc)

            completed = run_sprt.record_execution(manifest, args, now, now, 0, None)

            self.assertEqual(completed, 2)
            self.assertEqual(manifest["execution"]["status"], "failed")

    def test_missing_arbiter_results_report_no_faults(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            self.assertEqual(
                run_sprt.arbiter_faults(Path(directory) / "absent.json"), []
            )


if __name__ == "__main__":
    unittest.main()
