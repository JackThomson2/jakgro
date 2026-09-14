"""Small tests for the archive's source reconstruction and input checks."""

from __future__ import annotations

import gzip
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))
import archive_support as support
import prepare_stage


class ReconstructionTests(unittest.TestCase):
    def test_round_trip_changes_and_creates_files(self):
        source = {"src/a.rs": b"one\ntwo\nthree\n"}
        change = (
            "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n"
            "@@ -1,3 +1,3 @@\n one\n-two\n+second\n three\n"
            "diff --git a/src/b.rs b/src/b.rs\n--- /dev/null\n+++ b/src/b.rs\n"
            "@@ -0,0 +1,1 @@\n+new\n"
        )
        changed = support.patch_sources(source, change)
        self.assertEqual(changed, {"src/a.rs": b"one\nsecond\nthree\n", "src/b.rs": b"new\n"})
        self.assertEqual(support.patch_sources(changed, change, reverse=True), source)
        self.assertEqual(source, {"src/a.rs": b"one\ntwo\nthree\n"})

    def test_rejects_mismatched_context(self):
        change = "diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -1 +1 @@\n-wrong\n+new\n"
        with self.assertRaisesRegex(ValueError, "context mismatch"):
            support.patch_sources({"a": b"old\n"}, change)

    def test_rejects_escaping_paths(self):
        change = "diff --git a/../a b/../a\n--- a/../a\n+++ b/../a\n@@ -1 +1 @@\n-old\n+new\n"
        with self.assertRaisesRegex(ValueError, "unsafe patch path"):
            support.patch_sources({"../a": b"old\n"}, change)

    def test_every_archived_source_stage_has_the_expected_identity(self):
        provenance = support.read_json(support.ARCHIVE / "provenance.json")
        reconstructed = support.stages()
        self.assertEqual(set(reconstructed), set(provenance["source_stages"]))
        for name, source in reconstructed.items():
            with self.subTest(stage=name):
                self.assertEqual(support.source_digests(source), provenance["source_stages"][name])
        self.assertNotIn("src/engine/evaluation/memo.rs", reconstructed["base"])
        for stage in ("memo-8192", "memo-tagged"):
            self.assertEqual(reconstructed[stage]["src/engine/evaluation/weights.rs"],
                             reconstructed["base"]["src/engine/evaluation/weights.rs"])

    def test_preparation_refuses_existing_or_in_checkout_destinations(self):
        with self.assertRaisesRegex(ValueError, "outside the repository"):
            prepare_stage.prepare("base", support.ROOT / "test-stage")
        with tempfile.TemporaryDirectory() as temporary:
            with self.assertRaisesRegex(ValueError, "already exists"):
                prepare_stage.prepare("base", Path(temporary))

    def test_preparation_writes_verified_sources_and_corpora(self):
        with tempfile.TemporaryDirectory() as temporary:
            destination = Path(temporary) / "base"
            prepare_stage.prepare("base", destination)
            source = support.stages()["base"]
            for name, payload in source.items():
                self.assertEqual((destination / name).read_bytes(), payload)
            provenance = support.read_json(support.ARCHIVE / "provenance.json")
            for record in provenance["corpora"].values():
                compressed = (destination / record["path"]).read_bytes()
                self.assertEqual(support.digest(compressed), record["sha256"])
                self.assertEqual(len(gzip.decompress(compressed).splitlines()), record["rows"])

    def test_match_audit_rejects_a_changed_arbiter_result(self):
        import audit
        provenance = support.read_json(support.ARCHIVE / "provenance.json")
        real_read = audit.read_json

        def changed(path):
            result = real_read(path)
            if path.name == "safe-mobility-1-screen.arbiter.json":
                result["wins"] += 1
            return result

        with tempfile.TemporaryDirectory() as temporary, patch.object(audit, "read_json", changed):
            with self.assertRaisesRegex(ValueError, "W/D/L"):
                audit.audit_matches(provenance, Path(temporary))


if __name__ == "__main__":
    unittest.main()
