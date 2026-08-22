import argparse
import json
import tempfile
import unittest
from pathlib import Path

from tools import gate_null_search


class NullSearchGateTests(unittest.TestCase):
    def write_artifacts(self, root: Path) -> argparse.Namespace:
        candidate_hash = "a" * 64
        baseline_hash = "b" * 64
        contract = {
            "acceptance": {
                "same_profile_elo_floor": -35,
                "forcing_rate_retention_percent": 90,
            }
        }
        manifest = {
            "inputs": {
                "candidate": {
                    "name": "candidate",
                    "sha256": candidate_hash,
                    "aggression": 100,
                },
                "baseline": {
                    "name": "baseline",
                    "sha256": baseline_hash,
                    "aggression": 100,
                },
            },
            "settings": {"games": 96},
            "execution": {
                "status": "complete",
                "inputs_unchanged": True,
                "completed_games": 96,
            },
        }
        summary = {
            "engines": {"candidate": "candidate", "baseline": "baseline"},
            "games": 96,
            "confidence": {"elo": 0.0, "elo_ci95": [-140.0, 140.0]},
            "style": {
                "candidate": {"forcing_moves_per_100_moves": 39.0},
                "baseline": {"forcing_moves_per_100_moves": 40.0},
            },
        }
        style = self.style(candidate_hash, baseline_hash, 16)
        sacrifice = self.style(candidate_hash, baseline_hash, 4)
        benchmark = (
            "id,null_off_nodes,null_on_nodes,null_attempts,null_cutoffs\n"
            "active,1000,800,5,4\n"
            "forbidden,1000,1000,0,0\n"
        )
        values = {
            "contract.json": contract,
            "summary.json": summary,
            "manifest.json": manifest,
            "style.json": style,
            "sacrifice.json": sacrifice,
        }
        for name, value in values.items():
            (root / name).write_text(json.dumps(value), encoding="utf-8")
        (root / "benchmark.csv").write_text(benchmark, encoding="utf-8")
        return argparse.Namespace(
            contract=root / "contract.json",
            strength_summary=root / "summary.json",
            strength_manifest=root / "manifest.json",
            style_summary=root / "style.json",
            sacrifice_summary=root / "sacrifice.json",
            benchmark=root / "benchmark.csv",
            minimum_null_reduction=5.0,
        )

    @staticmethod
    def style(candidate_hash: str, baseline_hash: str, positions: int) -> dict:
        return {
            "inputs": {
                "candidate": {"sha256": candidate_hash},
                "baseline": {"sha256": baseline_hash},
            },
            "positions": [{} for _ in range(positions)],
            "gates": {
                "candidate_expected_moves": {"passed": True},
                "controls_preserved": {"passed": True},
            },
        }

    def test_complete_null_only_artifact_set_passes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            result = gate_null_search.gate_artifacts(
                self.write_artifacts(Path(directory))
            )

            self.assertTrue(result["passed"])
            self.assertEqual(result["strength_elo"], 0.0)
            self.assertEqual(
                result["benchmark"]["geometric_node_reduction_percent"], 20.0
            )

    def test_strength_and_forcing_floors_are_enforced(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            args = self.write_artifacts(root)
            summary = json.loads(args.strength_summary.read_text(encoding="utf-8"))
            summary["confidence"]["elo"] = -36.0
            args.strength_summary.write_text(json.dumps(summary), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "strength Elo"):
                gate_null_search.gate_artifacts(args)

            args = self.write_artifacts(root)
            summary = json.loads(args.strength_summary.read_text(encoding="utf-8"))
            summary["style"]["candidate"]["forcing_moves_per_100_moves"] = 35.0
            args.strength_summary.write_text(json.dumps(summary), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "forcing-rate retention"):
                gate_null_search.gate_artifacts(args)

    def test_benchmark_and_style_regressions_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            args = self.write_artifacts(root)
            args.benchmark.write_text(
                "id,null_off_nodes,null_on_nodes,null_attempts,null_cutoffs\n"
                "active,1000,990,5,4\n",
                encoding="utf-8",
            )
            with self.assertRaisesRegex(ValueError, "null node reduction"):
                gate_null_search.gate_artifacts(args)

            args = self.write_artifacts(root)
            style = json.loads(args.style_summary.read_text(encoding="utf-8"))
            style["gates"]["controls_preserved"]["passed"] = False
            args.style_summary.write_text(json.dumps(style), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "control gate"):
                gate_null_search.gate_artifacts(args)


if __name__ == "__main__":
    unittest.main()
