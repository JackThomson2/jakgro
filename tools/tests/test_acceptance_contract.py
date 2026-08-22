import json
import tempfile
import unittest
from pathlib import Path

from tools import validate_acceptance_contract


class AcceptanceContractTests(unittest.TestCase):
    def test_workspace_contract_is_complete_and_hash_locked(self) -> None:
        summary = validate_acceptance_contract.validate_contract(
            Path("tests/data/dual-channel-null-contract.json"), Path.cwd()
        )

        self.assertEqual(
            summary["baseline_commit"],
            "bc51beaa7e13ea47a9090168589e788984a90da7",
        )
        self.assertEqual(summary["suites"]["objective-personality"], 16)
        self.assertEqual(summary["suites"]["sacrifice"], 4)
        self.assertEqual(summary["suites"]["null-move"], 8)
        self.assertEqual(summary["acceptance"]["root_loss_cap_cp"], 120)
        self.assertEqual(summary["acceptance"]["null_forbidden_attempts"], 0)

    def test_hash_mismatch_is_rejected(self) -> None:
        source = Path("tests/data/dual-channel-null-contract.json")
        contract = json.loads(source.read_text(encoding="utf-8"))
        contract["suites"][0]["sha256"] = "0" * 64
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "contract.json"
            path.write_text(json.dumps(contract), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "hash mismatch"):
                validate_acceptance_contract.validate_contract(path, Path.cwd())

    def test_invalid_contract_metadata_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            suite = root / "suite.epd"
            suite.write_text(
                "8/8/8/8/8/8/8/K6k w - - 0 1 ; id bad ; class state ; "
                "null maybe ; depth 4 ; reason invalid\n",
                encoding="utf-8",
            )
            contract = {
                "schema_version": 1,
                "baseline_commit": "f" * 40,
                "suites": [
                    {
                        "path": "suite.epd",
                        "sha256": validate_acceptance_contract.sha256_file(suite),
                        "records": 1,
                        "kind": "null-move",
                    }
                ],
                "acceptance": {
                    "root_loss_cap_cp": 120,
                    "personality_required_hits": 16,
                    "sacrifice_required_positive_hits": 1,
                    "sacrifice_required_control_hits": 3,
                    "null_forbidden_attempts": 0,
                    "forcing_rate_retention_percent": 90,
                    "same_profile_elo_floor": -35,
                    "personality_cost_elo_floor": -75,
                },
            }
            path = root / "contract.json"
            path.write_text(json.dumps(contract), encoding="utf-8")

            with self.assertRaisesRegex(ValueError, "invalid null policy"):
                validate_acceptance_contract.validate_contract(path, root)


if __name__ == "__main__":
    unittest.main()
