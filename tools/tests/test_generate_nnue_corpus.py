import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from tools import generate_nnue_corpus


class GenerateNnueCorpusTests(unittest.TestCase):
    def run_generator(self, extra_arguments):
        commands = []

        def fake_run(command, log):
            commands.append(command)
            if command[1:2] == ["extract"]:
                Path(command[command.index("--out") + 1]).write_text(
                    "8/8/8/8/8/8/8/8 w - - 0 1;1;10\n8/8/8/8/8/8/8/8 b - - 0 1;0;-20\n",
                    encoding="utf-8",
                )
            else:
                Path(command[command.index("--pgn") + 1]).write_text("[Event \"x\"]\n", encoding="utf-8")

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in ("jakgro", "selfplay", "tune", "book.epd"):
                (root / name).write_bytes(name.encode())
            output = root / "corpus" / "rows.txt"
            arguments = [
                "--engine", str(root / "jakgro"), "--runner", str(root / "selfplay"),
                "--tune", str(root / "tune"), "--openings", str(root / "book.epd"),
                "--output", str(output), "--games-per-seed", "2", "--seeds", "7", "8",
                "--nodes", "1000", *extra_arguments,
            ]
            with mock.patch.object(generate_nnue_corpus, "run", fake_run):
                status = generate_nnue_corpus.main(arguments)
            self.assertEqual(status, 0)
            manifest = json.loads(output.with_suffix(".manifest.json").read_text(encoding="utf-8"))
            self.assertEqual(output.read_text(encoding="utf-8").count("\n"), 2)
        return commands, manifest

    def test_syzygy_path_reaches_both_sides_and_the_manifest(self) -> None:
        commands, manifest = self.run_generator(["--syzygy-path", "/tables/3-4-5:/tables/6"])

        selfplay_commands = [command for command in commands if command[1:2] != ["extract"]]
        self.assertEqual(len(selfplay_commands), 2)
        for command in selfplay_commands:
            self.assertEqual(command[command.index("--candidate-syzygy-path") + 1], "/tables/3-4-5:/tables/6")
            self.assertEqual(command[command.index("--baseline-syzygy-path") + 1], "/tables/3-4-5:/tables/6")
        self.assertEqual(manifest["settings"]["syzygy_path"], "/tables/3-4-5:/tables/6")
        self.assertEqual(manifest["rows"], 2)
        self.assertEqual(manifest["games"], 4)

    def test_without_a_syzygy_path_nothing_is_forwarded(self) -> None:
        commands, manifest = self.run_generator([])

        for command in commands:
            self.assertNotIn("--candidate-syzygy-path", command)
            self.assertNotIn("--baseline-syzygy-path", command)
        self.assertIsNone(manifest["settings"]["syzygy_path"])


if __name__ == "__main__":
    unittest.main()
