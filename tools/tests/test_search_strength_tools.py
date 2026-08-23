from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from typing import Any

from tools import gate_strength_personality, measure_search_efficiency, measure_style


class FakeEngine:
    def __init__(
        self,
        nodes: int,
        bestmove: str = "e2e4",
        nps: int = 1_000,
        timed_depth: int = 5,
    ) -> None:
        self.nodes = nodes
        self.bestmove = bestmove
        self.nps = nps
        self.timed_depth = timed_depth
        self.calls: list[tuple[int | None, int | None]] = []

    def measure(
        self,
        fixture: measure_style.Fixture,
        aggression: int,
        root_moves: frozenset[str] | None = None,
        depth: int | None = None,
        move_time_ms: int | None = None,
    ) -> measure_style.Observation:
        self.calls.append((depth, move_time_ms))
        if move_time_ms is not None:
            return measure_style.Observation(
                self.bestmove,
                "cp 10",
                self.timed_depth,
                self.nps * move_time_ms // 1_000,
                move_time_ms,
                self.nps,
            )
        if depth is None:
            elapsed_ms = max(1, fixture.nodes * 1_000 // self.nps)
            return measure_style.Observation(
                self.bestmove,
                "cp 10",
                self.timed_depth,
                fixture.nodes,
                elapsed_ms,
                self.nps,
            )
        return measure_style.Observation(
            self.bestmove,
            "cp 10",
            depth,
            self.nodes,
            max(1, self.nodes * 1_000 // self.nps),
            self.nps,
        )


class SearchEfficiencyTests(unittest.TestCase):
    def test_paired_channels_report_tree_speed_and_depth_gains(self) -> None:
        fixture = measure_style.Fixture(
            "start",
            "initiative",
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            100,
            {100: frozenset({"e2e4"})},
        )
        candidate_engine = FakeEngine(80, nps=1_200, timed_depth=6)
        baseline_engine = FakeEngine(100, nps=1_000, timed_depth=5)
        rows = measure_search_efficiency.measure_rows(
            candidate_engine,
            baseline_engine,
            [fixture],
            aggression=100,
            depth=4,
        )
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            candidate = root / "candidate"
            baseline = root / "baseline"
            suite = root / "suite.epd"
            candidate.write_bytes(b"candidate")
            baseline.write_bytes(b"baseline")
            suite.write_text("suite\n", encoding="utf-8")
            summary = measure_search_efficiency.summarize(
                rows,
                candidate,
                baseline,
                suite,
                aggression=100,
                depth=4,
                minimum_reduction=10.0,
                minimum_nps_gain=10.0,
                minimum_depth_gain=0.5,
                provenance={
                    "candidate_revision": "candidate-rev",
                    "baseline_revision": "baseline-rev",
                    "dependency_revision": "cozy-rev",
                    "build_profile": "release",
                },
            )

        self.assertEqual(candidate_engine.calls[0], (4, None))
        self.assertEqual(len(candidate_engine.calls), 9)
        self.assertEqual(summary["metrics"]["geometric_node_reduction_percent"], 20.0)
        self.assertEqual(summary["metrics"]["geometric_nps_gain_percent"], 20.0)
        self.assertEqual(summary["metrics"]["mean_completed_depth_gain"], 1.0)
        self.assertEqual(summary["metrics"]["active_positions"], 1)
        self.assertEqual(summary["metrics"]["repeatable_positions"], 1)
        self.assertEqual(summary["metrics"]["fixed_depth_candidate_nodes"], 80)
        self.assertEqual(summary["metrics"]["fixed_depth_baseline_nodes"], 100)
        self.assertTrue(summary["gates"]["all_positions_active"]["passed"])
        self.assertTrue(summary["gates"]["fixed_depth_repeatability"]["passed"])
        self.assertTrue(summary["passed"])
        self.assertRegex(summary["inputs"]["candidate"]["sha256"], r"^[0-9a-f]{64}$")

    def test_nonrepeatable_fixed_depth_results_fail_the_gate(self) -> None:
        fixture = measure_style.Fixture(
            "start",
            "initiative",
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            100,
            {100: frozenset({"e2e4"})},
        )
        candidate_engine = FakeEngine(80)
        baseline_engine = FakeEngine(100)
        original_measure = candidate_engine.measure
        fixed_depth_calls = 0

        def nonrepeatable_measure(
            fixture: measure_style.Fixture,
            aggression: int,
            root_moves: frozenset[str] | None = None,
            depth: int | None = None,
            move_time_ms: int | None = None,
        ) -> measure_style.Observation:
            nonlocal fixed_depth_calls
            observation = original_measure(
                fixture, aggression, root_moves, depth, move_time_ms
            )
            if depth is not None:
                fixed_depth_calls += 1
                if fixed_depth_calls == 2:
                    return measure_style.Observation(
                        observation.bestmove,
                        observation.score,
                        observation.depth,
                        observation.nodes + 1,
                        observation.elapsed_ms,
                        observation.nps,
                    )
            return observation

        candidate_engine.measure = nonrepeatable_measure  # type: ignore[method-assign]
        rows = measure_search_efficiency.measure_rows(
            candidate_engine,
            baseline_engine,
            [fixture],
            aggression=100,
            depth=4,
            samples=1,
        )
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            candidate = root / "candidate"
            baseline = root / "baseline"
            suite = root / "suite.epd"
            candidate.write_bytes(b"candidate")
            baseline.write_bytes(b"baseline")
            suite.write_text("suite\n", encoding="utf-8")
            summary = measure_search_efficiency.summarize(
                rows,
                candidate,
                baseline,
                suite,
                aggression=100,
                depth=4,
                minimum_reduction=-100.0,
            )

        self.assertFalse(summary["gates"]["fixed_depth_repeatability"]["passed"])
        self.assertEqual(summary["metrics"]["nonrepeatable_positions"], ["start"])
        self.assertFalse(summary["passed"])


class StrengthPersonalityGateTests(unittest.TestCase):
    def write_json(self, path: Path, value: object) -> None:
        path.write_text(json.dumps(value), encoding="utf-8")

    def write_match(
        self,
        root: Path,
        label: str,
        candidate_name: str,
        baseline_name: str,
        candidate_aggression: int,
        baseline_aggression: int,
        candidate_hash: str,
        baseline_hash: str,
        elo: float,
        lower: float | None,
    ) -> tuple[Path, Path]:
        manifest = root / f"{label}.manifest.json"
        self.write_json(
            manifest,
            {
                "execution": {
                    "status": "complete",
                    "inputs_unchanged": True,
                    "completed_games": 48,
                },
                "inputs": {
                    "candidate": {
                        "name": candidate_name,
                        "aggression": candidate_aggression,
                        "sha256": candidate_hash,
                    },
                    "baseline": {
                        "name": baseline_name,
                        "aggression": baseline_aggression,
                        "sha256": baseline_hash,
                    },
                },
            },
        )
        summary = root / f"{label}.summary.json"
        self.write_json(
            summary,
            {
                "games": 48,
                "inputs": {
                    "manifest_sha256": gate_strength_personality.sha256_file(manifest),
                },
                "engines": {
                    "candidate": candidate_name,
                    "baseline": baseline_name,
                },
                "confidence": {
                    "elo": elo,
                    "elo_ci95": [lower, 200.0] if lower is not None else [None, None],
                },
                "style": {
                    "candidate": {"forcing_moves_per_100_moves": 12.0},
                    "baseline": {"forcing_moves_per_100_moves": 10.0},
                },
            },
        )
        return summary, manifest

    def write_artifacts(self, root: Path) -> dict[str, Any]:
        candidate = root / "candidate"
        baseline = root / "baseline"
        candidate.write_bytes(b"candidate")
        baseline.write_bytes(b"baseline")
        candidate_hash = gate_strength_personality.sha256_file(candidate)
        baseline_hash = gate_strength_personality.sha256_file(baseline)
        matches = {
            "objective": self.write_match(
                root,
                "objective",
                "Candidate-A0",
                "Baseline-A0",
                0,
                0,
                candidate_hash,
                baseline_hash,
                10.0,
                -40.0,
            ),
            "same_profile": self.write_match(
                root,
                "same-profile",
                "Candidate-A100",
                "Baseline-A100",
                100,
                100,
                candidate_hash,
                baseline_hash,
                20.0,
                -30.0,
            ),
            "personality": self.write_match(
                root,
                "personality",
                "Candidate-A100",
                "Candidate-A0",
                100,
                0,
                candidate_hash,
                candidate_hash,
                -50.0,
                -120.0,
            ),
            "baseline_personality": self.write_match(
                root,
                "baseline-personality",
                "Baseline-A100",
                "Baseline-A0",
                100,
                0,
                baseline_hash,
                baseline_hash,
                -60.0,
                -130.0,
            ),
        }
        contract = root / "contract.json"
        self.write_json(
            contract,
            {
                "matches": {
                    "objective": {
                        "candidate_aggression": 0,
                        "baseline_aggression": 0,
                        "minimum_games": 48,
                        "minimum_elo": -100,
                        "minimum_elo_ci95_lower": -100,
                    },
                    "same_profile": {
                        "candidate_aggression": 100,
                        "baseline_aggression": 100,
                        "minimum_games": 48,
                        "minimum_elo": -100,
                        "minimum_elo_ci95_lower": -100,
                    },
                    "personality": {
                        "candidate_aggression": 100,
                        "baseline_aggression": 0,
                        "minimum_games": 48,
                    },
                    "baseline_personality": {
                        "candidate_aggression": 100,
                        "baseline_aggression": 0,
                        "minimum_games": 48,
                    },
                },
                "personality_comparison": {"minimum_elo_delta": -20},
                "deterministic": {
                    "minimum_forcing_retention_percent": 90,
                    "maximum_root_loss_cp": 45,
                },
                "efficiency": {"minimum_node_reduction_percent": -10},
            },
        )
        style = root / "style.json"
        self.write_json(
            style,
            {
                "inputs": {
                    "candidate": {"sha256": candidate_hash},
                    "baseline": {"sha256": baseline_hash},
                },
                "gates": {
                    "candidate_expected_moves": {"passed": True},
                    "controls_preserved": {"passed": True},
                },
            },
        )
        acceptance_paths = []
        for label, losses in (("objective-acceptance", [0, 44]), ("sacrifice", [37, 0])):
            path = root / f"{label}.json"
            self.write_json(
                path,
                {
                    "inputs": {"engine": {"sha256": candidate_hash}},
                    "positions": [{"root_loss_cp": loss} for loss in losses],
                    "passed": True,
                },
            )
            acceptance_paths.append(path)
        efficiency = root / "efficiency.json"
        self.write_json(
            efficiency,
            {
                "inputs": {
                    "candidate": {"sha256": candidate_hash},
                    "baseline": {"sha256": baseline_hash},
                },
                "metrics": {
                    "active_positions": 8,
                    "geometric_node_reduction_percent": 5.0,
                },
            },
        )
        return {
            "contract": contract,
            "matches": matches,
            "style": style,
            "acceptance": acceptance_paths,
            "efficiency": efficiency,
        }

    def gate(self, artifacts: dict[str, Any]) -> dict[str, Any]:
        return gate_strength_personality.gate_artifacts(
            artifacts["contract"],
            artifacts["matches"],
            artifacts["style"],
            artifacts["acceptance"],
            artifacts["efficiency"],
        )

    def test_complete_cross_channel_artifact_set_passes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            result = self.gate(self.write_artifacts(Path(directory)))

        self.assertTrue(result["passed"])
        self.assertEqual(result["style"]["forcing_retention_percent"], 120.0)
        self.assertEqual(result["acceptance"][0]["maximum_root_loss_cp"], 44)
        self.assertEqual(result["personality_comparison"]["elo_delta"], 10.0)

    def test_confidence_and_efficiency_regressions_fail_the_gate(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            artifacts = self.write_artifacts(Path(directory))
            personality = artifacts["matches"]["personality"][0]
            summary = json.loads(personality.read_text(encoding="utf-8"))
            summary["confidence"] = {"elo": -300.0, "elo_ci95": [None, None]}
            self.write_json(personality, summary)
            efficiency = artifacts["efficiency"]
            value = json.loads(efficiency.read_text(encoding="utf-8"))
            value["metrics"]["geometric_node_reduction_percent"] = -11.0
            self.write_json(efficiency, value)

            result = self.gate(artifacts)

        self.assertFalse(result["passed"])
        self.assertFalse(result["personality_comparison"]["passed"])
        self.assertLess(result["personality_comparison"]["elo_delta"], -20)
        self.assertFalse(result["efficiency"]["passed"])

    def test_binary_identity_mismatch_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            artifacts = self.write_artifacts(Path(directory))
            acceptance = artifacts["acceptance"][0]
            value = json.loads(acceptance.read_text(encoding="utf-8"))
            value["inputs"]["engine"]["sha256"] = "0" * 64
            self.write_json(acceptance, value)

            with self.assertRaisesRegex(ValueError, "candidate binary hash"):
                self.gate(artifacts)


if __name__ == "__main__":
    unittest.main()
