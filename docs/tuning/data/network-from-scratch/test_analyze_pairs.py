import gzip
import importlib.util
from pathlib import Path
import tempfile
import unittest


SPEC = importlib.util.spec_from_file_location(
    "network_from_scratch_analyze_pairs", Path(__file__).with_name("analyze_pairs.py")
)
ANALYZER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ANALYZER)


def game(
    round_number, white, black, opening="e4", result="1-0",
    moves="Qh5+ Nc6", termination="normal",
):
    return (
        '[Event "test"]\n'
        f'[Round "{round_number}"]\n'
        f'[White "{white}"]\n'
        f'[Black "{black}"]\n'
        f'[Result "{result}"]\n'
        f'[Termination "{termination}"]\n'
        '[PlyCount "4"]\n\n'
        f"1. {opening} {{book}} e5 {{book}} 2. {moves} {result}\n\n"
    )


class PairAnalysisTests(unittest.TestCase):
    def analyze(self, text, compressed=False, allow_incomplete_pairs=False):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / ("match.pgn.gz" if compressed else "match.pgn")
            raw = text.encode()
            path.write_bytes(gzip.compress(raw) if compressed else raw)
            return ANALYZER.summarize(
                path, book_plies=2, bootstrap=20,
                allow_incomplete_pairs=allow_incomplete_pairs,
            )

    def test_rounds_not_missing_fens_identify_pairs_in_completion_order(self):
        text = (
            game(2, "Baseline", "Candidate", "d4")
            + game(1, "Candidate", "Baseline")
            + game(2, "Candidate", "Baseline", "d4")
            + game(1, "Baseline", "Candidate")
        )
        result = self.analyze(text)
        self.assertEqual(result["strength"]["pairs"], 2)
        self.assertEqual(result["strength"]["elo"], 0)
        self.assertEqual(result["strength"]["interval_status"], "degenerate")
        self.assertIsNone(result["strength"]["elo_ci95"])
        self.assertNotIn("los_percent", result["strength"])
        self.assertEqual(result["style"]["candidate_moves"], 4)
        self.assertEqual(result["style"]["baseline_moves"], 4)
        self.assertEqual(result["style"]["checks"]["rate_ratio"], 1)
        self.assertEqual(result["style"]["forcing"]["rate_ratio"], 1)
        self.assertEqual(result["fault_terminations"], {})
        archived = self.analyze(text, compressed=True)
        self.assertEqual(archived["uncompressed_pgn_sha256"], result["uncompressed_pgn_sha256"])
        self.assertEqual(archived["strength"], result["strength"])
        self.assertEqual(archived["style"], result["style"])

    def test_missing_partner_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "two games"):
            self.analyze(game(1, "Candidate", "Baseline"))

    def test_sprt_orphans_are_reported_and_still_audited_for_faults(self):
        text = (
            game(2, "Baseline", "Candidate", termination="abandoned")
            + game(1, "Candidate", "Baseline")
            + game(1, "Baseline", "Candidate")
        )
        result = self.analyze(text, allow_incomplete_pairs=True)
        self.assertEqual(result["recorded_games"], 3)
        self.assertEqual(result["strength"]["games"], 2)
        self.assertEqual(result["unpaired_games"][0]["round"], 2)
        self.assertEqual(result["fault_terminations"], {"abandoned": 1})

    def test_an_orphan_alone_cannot_estimate_strength(self):
        with self.assertRaisesRegex(ValueError, "no complete pairs"):
            self.analyze(game(1, "Candidate", "Baseline"), allow_incomplete_pairs=True)

    def test_singular_bootstrap_samples_do_not_produce_a_finite_interval(self):
        text = (
            game(1, "Candidate", "Baseline")
            + game(1, "Baseline", "Candidate", moves="Qh5 Nc6")
            + game(2, "Candidate", "Baseline")
            + game(2, "Baseline", "Candidate")
        )
        checks = self.analyze(text)["style"]["checks"]
        self.assertEqual(checks["rate_ratio"], 2)
        self.assertGreater(checks["bootstrap_replicates"]["unbounded"], 0)
        self.assertEqual(sum(checks["bootstrap_replicates"].values()), 20)
        self.assertIsNone(checks["bootstrap_ci95"])

    def test_no_check_events_is_an_undefined_ratio(self):
        text = (
            game(1, "Candidate", "Baseline", moves="Qh5 Nc6")
            + game(1, "Baseline", "Candidate", moves="Qh5 Nc6")
        )
        checks = self.analyze(text)["style"]["checks"]
        self.assertEqual(checks["ratio_status"], "undefined")
        self.assertEqual(checks["bootstrap_replicates"]["undefined"], 20)
        self.assertIsNone(checks["rate_ratio"])
        self.assertIsNone(checks["bootstrap_ci95"])

    def test_candidate_checks_as_black_are_counted_for_the_candidate(self):
        text = (
            game(1, "Candidate", "Baseline")
            + game(1, "Baseline", "Candidate", moves="Qh5+ Nf6+")
        )
        checks = self.analyze(text)["style"]["checks"]
        self.assertEqual(checks["candidate"], 2)
        self.assertEqual(checks["baseline"], 1)
        self.assertEqual(checks["rate_ratio"], 2)

    def test_mismatched_openings_are_rejected(self):
        with self.assertRaisesRegex(ValueError, "different openings"):
            self.analyze(
                game(1, "Candidate", "Baseline")
                + game(1, "Baseline", "Candidate", "d4")
            )

    def test_colors_must_reverse(self):
        with self.assertRaisesRegex(ValueError, "reverse colors"):
            self.analyze(game(1, "Candidate", "Baseline") * 2)

    def test_book_length_must_match_the_recorded_comments(self):
        with self.assertRaisesRegex(ValueError, "book plies"):
            self.analyze(
                (game(1, "Candidate", "Baseline") + game(1, "Baseline", "Candidate"))
                .replace("{book}", "{0.00}", 1)
            )


if __name__ == "__main__":
    unittest.main()
