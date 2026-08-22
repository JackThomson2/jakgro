from __future__ import annotations
import tempfile
import unittest
from pathlib import Path

from tools import measure_acceptance, measure_style


class ContractParserTests(unittest.TestCase):
    def test_frozen_contracts_parse_expected_profiles_and_loss_limits(self) -> None:
        objective = measure_acceptance.parse_contract_suite(
            Path("tests/data/objective-personality-contract.epd")
        )
        sacrifices = measure_acceptance.parse_contract_suite(
            Path("tests/data/sacrifice-acceptance-contract.epd")
        )

        self.assertTrue(objective)
        self.assertTrue(sacrifices)
        self.assertTrue(all(set(fixture.expected) == {0, 100} for fixture in objective))
        self.assertTrue(all(set(fixture.expected) == {100} for fixture in sacrifices))
        self.assertTrue(all(0 <= fixture.maximum_loss_cp <= 120 for fixture in objective + sacrifices))

    def test_parser_rejects_invalid_maximum_loss(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            suite = Path(directory) / "contract.epd"
            suite.write_text(
                "8/8/8/8/8/8/4K3/7k w - - 0 1 ; "
                "id invalid ; category safety ; nodes 10 ; obm e2e3 ; "
                "bm100 e2e3 ; maxloss 121 ; gate control ; motif kings\n",
                encoding="utf-8",
            )

            with self.assertRaisesRegex(ValueError, "maxloss"):
                measure_acceptance.parse_contract_suite(suite)


class ScoreTests(unittest.TestCase):
    def test_centipawn_and_mate_scores_share_an_ordered_scale(self) -> None:
        self.assertEqual(measure_acceptance.score_to_cp("cp -42"), -42)
        self.assertGreater(
            measure_acceptance.score_to_cp("mate 3"),
            measure_acceptance.score_to_cp("cp 9000"),
        )
        self.assertLess(
            measure_acceptance.score_to_cp("mate -2"),
            measure_acceptance.score_to_cp("cp -9000"),
        )
        self.assertGreater(
            measure_acceptance.score_to_cp("mate 2"),
            measure_acceptance.score_to_cp("mate 4"),
        )

    def test_invalid_score_is_rejected(self) -> None:
        with self.assertRaisesRegex(ValueError, "unsupported UCI score"):
            measure_acceptance.score_to_cp("lowerbound cp 10")


class FakeEngine:
    def __init__(self, selected_score: str = "cp 5") -> None:
        self.selected_score = selected_score
        self.calls: list[tuple[int, frozenset[str] | None]] = []

    def measure(
        self,
        fixture: measure_style.Fixture,
        aggression: int,
        root_moves: frozenset[str] | None = None,
    ) -> measure_style.Observation:
        self.calls.append((aggression, root_moves))
        if root_moves == frozenset({"e2e4"}):
            return measure_style.Observation("e2e4", "cp 25", 6, fixture.nodes)
        if root_moves == frozenset({"g1f3"}):
            return measure_style.Observation("g1f3", self.selected_score, 6, fixture.nodes)
        if aggression == 100:
            return measure_style.Observation("g1f3", "cp 10", 6, fixture.nodes)
        return measure_style.Observation("e2e4", "cp 25", 6, fixture.nodes)


class AcceptanceMeasurementTests(unittest.TestCase):
    def fixture(self, maximum_loss_cp: int = 20) -> measure_acceptance.ContractFixture:
        return measure_acceptance.ContractFixture(
            identifier="opening-choice",
            category="initiative",
            fen="rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            nodes=100,
            objective_moves=frozenset({"e2e4"}),
            expected={0: frozenset({"e2e4"}), 100: frozenset({"g1f3"})},
            maximum_loss_cp=maximum_loss_cp,
            gate="objective",
            motif="development",
        )

    def test_measurement_uses_objective_research_for_observed_styled_move(self) -> None:
        engine = FakeEngine(selected_score="cp 5")

        positions = measure_acceptance.measure_positions(engine, [self.fixture()])

        self.assertEqual(positions[0]["root_loss_cp"], 20)
        self.assertTrue(positions[0]["passed"])
        self.assertIn((0, frozenset({"e2e4"})), engine.calls)
        self.assertIn((0, frozenset({"g1f3"})), engine.calls)

    def test_summary_binds_inputs_and_reports_loss_failure(self) -> None:
        positions = measure_acceptance.measure_positions(
            FakeEngine(selected_score="cp 4"),
            [self.fixture()],
        )
        with tempfile.TemporaryDirectory() as directory:
            engine = Path(directory) / "engine"
            suite = Path(directory) / "suite.epd"
            engine.write_bytes(b"engine")
            suite.write_text("suite\n", encoding="utf-8")

            summary = measure_acceptance.summarize(positions, engine, suite)

        self.assertFalse(summary["passed"])
        self.assertEqual(
            summary["gates"]["root_loss"]["failed_positions"],
            ["opening-choice"],
        )
        self.assertRegex(summary["inputs"]["engine"]["sha256"], r"^[0-9a-f]{64}$")


if __name__ == "__main__":
    unittest.main()
