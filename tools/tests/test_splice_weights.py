import tempfile
import unittest
from pathlib import Path

from tools import splice_weights

FIT = """\
// Fitted by `tune fit`. Paste into the files named below.

// ---- src/engine/evaluation/weights.rs ----
const PAWN: ScorePair = ScorePair::new(94, 121);
const KNIGHT_MOBILITY: [ScorePair; 2] = [
    ScorePair::new(-2, 0),
    ScorePair::new(4, 9),
];
const KING_DANGER_BY_BUCKET: [ScorePair; 2] = [
    ScorePair::new(1, -1),
    ScorePair::new(2, -2),
];
const STORM_KING_FILE_BY_DISTANCE: [ScorePair; 2] = [
    ScorePair::new(3, 4),
    ScorePair::new(5, 6),
];

// ---- src/engine/evaluation/placement.rs ----
static PAWN: Table = Table {
    middle_game: [
        0, 0, 0, 0, 0, 0, 0, 0, //
        1, 1, 1, 1, 1, 1, 1, 1, //
    ],
    end_game: [
        0, 0, 0, 0, 0, 0, 0, 0, //
        2, 2, 2, 2, 2, 2, 2, 2, //
    ],
};
"""

WEIGHTS = """\
/// A pawn.
const PAWN: ScorePair = ScorePair::new(100, 100);
const KNIGHT: ScorePair = ScorePair::new(320, 320);
const KNIGHT_MOBILITY: [ScorePair; 2] = [
    ScorePair::new(0, 0),
    ScorePair::new(0, 0),
];
/// Named length, kept.
const KING_DANGER_BY_BUCKET: [ScorePair; KING_DANGER_BUCKETS] =
    [ScorePair::new(0, 0); KING_DANGER_BUCKETS];
const STORM_KING_FILE_BY_DISTANCE: [ScorePair; 2] = [ScorePair::new(0, 0); 2];
"""

PLACEMENT = """\
static PAWN: Table = Table {
    middle_game: [
        9, 9, 9, 9, 9, 9, 9, 9, //
        9, 9, 9, 9, 9, 9, 9, 9, //
    ],
    end_game: [
        9, 9, 9, 9, 9, 9, 9, 9, //
        9, 9, 9, 9, 9, 9, 9, 9, //
    ],
};

static KNIGHT: Table = Table {
    middle_game: [
        7, 7, 7, 7, 7, 7, 7, 7, //
    ],
    end_game: [
        7, 7, 7, 7, 7, 7, 7, 7, //
    ],
};
"""


class ParseTests(unittest.TestCase):
    def test_declarations_are_grouped_by_banner_in_order(self) -> None:
        parsed = splice_weights.parse_fit(FIT)

        self.assertEqual(list(parsed), ["src/engine/evaluation/weights.rs", "src/engine/evaluation/placement.rs"])
        self.assertEqual(
            [(kind, name) for kind, name, _ in parsed["src/engine/evaluation/weights.rs"]],
            [
                ("scalar", "PAWN"),
                ("array", "KNIGHT_MOBILITY"),
                ("array", "KING_DANGER_BY_BUCKET"),
                ("array", "STORM_KING_FILE_BY_DISTANCE"),
            ],
        )
        self.assertEqual(
            [(kind, name) for kind, name, _ in parsed["src/engine/evaluation/placement.rs"]],
            [("table", "PAWN")],
        )

    def test_an_array_with_the_wrong_entry_count_is_refused(self) -> None:
        text = "// ---- a.rs ----\nconst X: [ScorePair; 3] = [\n    ScorePair::new(1, 1),\n];\n"

        with self.assertRaisesRegex(splice_weights.SpliceError, "declares 3 entries but lists 1"):
            splice_weights.parse_fit(text)

    def test_text_before_a_banner_is_refused(self) -> None:
        with self.assertRaisesRegex(splice_weights.SpliceError, "precedes any file banner"):
            splice_weights.parse_fit("const X: ScorePair = ScorePair::new(1, 1);\n")


class SpliceTests(unittest.TestCase):
    def test_every_named_declaration_is_replaced_and_the_rest_left_alone(self) -> None:
        parsed = splice_weights.parse_fit(FIT)

        spliced, replaced = splice_weights.splice(WEIGHTS, parsed["src/engine/evaluation/weights.rs"])

        self.assertEqual(replaced, ["PAWN", "KNIGHT_MOBILITY", "KING_DANGER_BY_BUCKET", "STORM_KING_FILE_BY_DISTANCE"])
        self.assertIn("/// A pawn.\nconst PAWN: ScorePair = ScorePair::new(94, 121);\n", spliced)
        self.assertIn("const KNIGHT: ScorePair = ScorePair::new(320, 320);\n", spliced)
        self.assertIn(
            "const KNIGHT_MOBILITY: [ScorePair; 2] = [\n    ScorePair::new(-2, 0),\n    ScorePair::new(4, 9),\n];\n",
            spliced,
        )

    def test_a_named_length_survives_and_a_one_line_zero_array_is_expanded(self) -> None:
        parsed = splice_weights.parse_fit(FIT)

        spliced, _ = splice_weights.splice(WEIGHTS, parsed["src/engine/evaluation/weights.rs"])

        self.assertIn(
            "/// Named length, kept.\nconst KING_DANGER_BY_BUCKET: [ScorePair; KING_DANGER_BUCKETS] = [\n"
            "    ScorePair::new(1, -1),\n    ScorePair::new(2, -2),\n];\n",
            spliced,
        )
        self.assertIn(
            "const STORM_KING_FILE_BY_DISTANCE: [ScorePair; 2] = [\n    ScorePair::new(3, 4),\n    ScorePair::new(5, 6),\n];\n",
            spliced,
        )
        self.assertNotIn("[ScorePair::new(0, 0); 2]", spliced)

    def test_a_table_is_replaced_whole_and_its_neighbour_kept(self) -> None:
        parsed = splice_weights.parse_fit(FIT)

        spliced, replaced = splice_weights.splice(PLACEMENT, parsed["src/engine/evaluation/placement.rs"])

        self.assertEqual(replaced, ["PAWN"])
        self.assertIn("        1, 1, 1, 1, 1, 1, 1, 1, //\n", spliced)
        self.assertNotIn("        9, 9, 9, 9, 9, 9, 9, 9, //\n", spliced)
        self.assertIn("static KNIGHT: Table = Table {\n    middle_game: [\n        7, 7, 7, 7, 7, 7, 7, 7, //\n", spliced)

    def test_an_unknown_or_duplicated_name_is_refused(self) -> None:
        with self.assertRaisesRegex(splice_weights.SpliceError, "MISSING is not declared"):
            splice_weights.splice(WEIGHTS, [("scalar", "MISSING", ["const MISSING: ScorePair = ScorePair::new(1, 1);"])])

        doubled = WEIGHTS + "const PAWN: ScorePair = ScorePair::new(1, 1);\n"
        with self.assertRaisesRegex(splice_weights.SpliceError, "PAWN is declared 2 times"):
            splice_weights.splice(doubled, [("scalar", "PAWN", ["const PAWN: ScorePair = ScorePair::new(1, 1);"])])


class MainTests(unittest.TestCase):
    def test_main_writes_both_files_and_a_dry_run_writes_neither(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            weights = root / "src/engine/evaluation/weights.rs"
            placement = root / "src/engine/evaluation/placement.rs"
            weights.parent.mkdir(parents=True)
            weights.write_text(WEIGHTS)
            placement.write_text(PLACEMENT)
            fit = root / "fit.txt"
            fit.write_text(FIT)

            self.assertEqual(splice_weights.main([str(fit), "--root", str(root), "--dry-run"]), 0)
            self.assertEqual(weights.read_text(), WEIGHTS)
            self.assertEqual(placement.read_text(), PLACEMENT)

            self.assertEqual(splice_weights.main([str(fit), "--root", str(root)]), 0)
            self.assertIn("ScorePair::new(94, 121)", weights.read_text())
            self.assertIn("        2, 2, 2, 2, 2, 2, 2, 2, //\n", placement.read_text())

    def test_main_reports_a_missing_target_file(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fit = root / "fit.txt"
            fit.write_text(FIT)

            self.assertEqual(splice_weights.main([str(fit), "--root", str(root)]), 1)


if __name__ == "__main__":
    unittest.main()
