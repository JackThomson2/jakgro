import contextlib
from array import array
from dataclasses import replace
import gzip
import io
import json
import os
from pathlib import Path
import struct
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from tools import nnue_data as data
from tools import nnue_format as fmt
from tools import train_nnue as train

HELPER_ENV = os.environ.get("JAKGRO_NNUE_DATA")
HELPER = Path(HELPER_ENV).resolve() if HELPER_ENV else None
if HELPER is not None and not HELPER.is_file():
    raise RuntimeError("JAKGRO_NNUE_DATA does not name a built nnue-data executable")

FEN = "4k3/8/5n2/8/8/8/2P5/4K3 w - - 0 1"
SAMPLE = data.Sample(FEN, " ".join(FEN.split()[:4]), 0, 1.0, 150.0,
                     (1546, 1860, 2029, 2300), (1621, 1860, 1970, 2300))


def zero_network(bias=0):
    return fmt.Network((0,) * fmt.HIDDEN, array("h", [0]) * (fmt.INPUTS * fmt.HIDDEN),
                       (0,) * (2 * fmt.HIDDEN), bias)


def sample_with_pawn(file):
    fen = f"4k3/8/5n2/8/8/8/{file}P{7-file}/4K3 w - - 0 1"
    # Only files 1..6 are used, avoiding zero FEN run lengths.
    return data.Sample(fen, " ".join(fen.split()[:4]), 0, 1.0, 150.0,
                       (1544 + file, 1860, 2029, 2300),
                       (1621, 1860, 1968 + file, 2300))


class FormatTests(unittest.TestCase):
    def test_signed_rounding_and_division(self):
        self.assertEqual([fmt.round_away(value) for value in (-1.5, -0.5, -0.49, 0.49, 0.5, 1.5)],
                         [-2, -1, 0, 0, 1, 2])
        self.assertEqual([fmt.trunc_div(value, 16320) for value in (-16321, -16320, -16319, 16319, 16320)],
                         [-1, -1, 0, 0, 1])
        for value in (float("nan"), float("inf"), float("-inf")):
            with self.assertRaises(ValueError):
                fmt.round_away(value)
        with self.assertRaises(ValueError):
            fmt.trunc_div(1, 0)

    def test_binary_round_trip_and_oracle_clipping(self):
        network = zero_network(-16321)
        self.assertEqual(network.infer(SAMPLE.white, SAMPLE.black, 0)["cp"], -1)
        inputs = array("h", network.input_weights)
        inputs[SAMPLE.white[0] * fmt.HIDDEN] = 300
        inputs[SAMPLE.black[0] * fmt.HIDDEN + 1] = -500
        network = fmt.Network((0,) * fmt.HIDDEN, inputs, (640,) + (0,) * 255, -16321)
        payload = network.to_bytes()
        self.assertEqual(len(payload), fmt.FILE_BYTES)
        restored = fmt.Network.from_bytes(payload)
        self.assertEqual(restored.to_bytes(), payload)
        white = restored.infer(SAMPLE.white, SAMPLE.black, 0)
        black = restored.infer(SAMPLE.white, SAMPLE.black, 1)
        self.assertEqual(white["white"][0], 300)
        self.assertEqual(white["black"][1], -500)
        self.assertEqual(white["cp"], 8)
        self.assertEqual(black["cp"], -1)

    def test_headers_corruption_and_lengths_are_rejected(self):
        blob = zero_network().to_bytes()
        for length in (0, 8, 47, 48, len(blob) - 1):
            with self.subTest(length=length), self.assertRaisesRegex(ValueError, "length"):
                fmt.Network.from_bytes(blob[:length])
        with self.assertRaisesRegex(ValueError, "length"):
            fmt.Network.from_bytes(blob + b"x")
        for offset in (0, 8, 12, 16, 20, 24, 28, 32, 36, 40, 48, len(blob) - 1):
            changed = bytearray(blob)
            changed[offset] ^= 1
            with self.subTest(offset=offset), self.assertRaises(ValueError):
                fmt.Network.from_bytes(bytes(changed))

    def test_storage_and_output_overflow_bounds(self):
        network = zero_network()
        with self.assertRaises(ValueError):
            replace(network, hidden_bias=(32768,) * fmt.HIDDEN).to_bytes()
        with self.assertRaises(ValueError):
            replace(network, output_bias=-(1 << 31)).to_bytes()
        output = (-32768,) * (2 * fmt.HIDDEN)
        boundary = (1 << 31) - 1 - 255 * 256 * 32768
        replace(network, output_weights=output, output_bias=boundary).validate()
        replace(network, output_weights=output, output_bias=-boundary).validate()
        with self.assertRaisesRegex(ValueError, "arithmetic"):
            replace(network, output_weights=output, output_bias=boundary + 1).validate()
        self.assertEqual(replace(network, output_bias=(1 << 31) - 1).infer(SAMPLE.white, SAMPLE.black, 0)["cp"], 16000)

    def test_invalid_feature_vectors_are_rejected(self):
        for values in ([], [0], [1, 1], [2, 1], [-1, 2], [0, 12288], [0, 768], [False, 1]):
            with self.subTest(values=values), self.assertRaises(ValueError):
                fmt.features(values)


class DatasetTests(unittest.TestCase):
    def test_records_round_trip_without_label_reorientation(self):
        black = replace(SAMPLE, fen=FEN.replace(" w ", " b "), key=SAMPLE.key.replace(" w ", " b "),
                        stm=1, outcome=0.0, teacher=-100.0)
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "data.tsv"
            data.write_records(path, [SAMPLE, black])
            self.assertEqual(data.read_records(path), [SAMPLE, black])

    def test_malformed_prepared_rows_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "data.tsv"
            data.write_records(path, [SAMPLE])
            original = path.read_text()
            for old, new in (("data-v1", "data-v2"), ("\t1.0\t", "\tnan\t"),
                             ("\t150.0\t", "\tinf\t"), ("1546,1860", "1546,1546"),
                             ("\t0\t1.0", "\t2\t1.0")):
                path.write_text(original.replace(old, new))
                with self.subTest(new=new), self.assertRaises(ValueError):
                    data.read_records(path)
            path.write_text(original[:-1])
            with self.assertRaisesRegex(ValueError, "length"):
                data.read_records(path)

    def test_canonical_and_feature_overlap_including_turn_and_mirror(self):
        data.check_split([SAMPLE], [sample_with_pawn(3)])
        with self.assertRaisesRegex(ValueError, "canonical"):
            data.check_split([SAMPLE], [SAMPLE])
        for changed in (
            replace(SAMPLE, key=SAMPLE.key + "x"),
            replace(SAMPLE, key=SAMPLE.key.replace(" w ", " b "), stm=1),
            replace(SAMPLE, key="mirrored", white=SAMPLE.black, black=SAMPLE.white, stm=1),
        ):
            with self.assertRaisesRegex(ValueError, "feature-identical"):
                data.check_split([SAMPLE], [changed])

    def test_deduplication_is_explicit_and_counts_drops(self):
        changed = replace(SAMPLE, key="same-features-different-rights")
        with self.assertRaises(ValueError):
            data.unique_records([SAMPLE, SAMPLE], False)
        kept, dropped = data.unique_records([SAMPLE, SAMPLE, changed], True)
        self.assertEqual(kept, [SAMPLE])
        self.assertEqual(dropped, {"canonical_duplicate": 1, "feature_duplicate": 1})
        with self.assertRaisesRegex(ValueError, "empty"):
            data.check_split([], [SAMPLE])

    def test_decompression_has_an_explicit_size_bound(self):
        with tempfile.TemporaryDirectory() as temporary:
            source, output = Path(temporary) / "input.gz", Path(temporary) / "expanded"
            source.write_bytes(gzip.compress(b"a" * 1000))
            with patch.object(data, "MAX_EXPANDED_BYTES", 10), self.assertRaisesRegex(ValueError, "exceeds"):
                data.expand(source, output)


class OptionTests(unittest.TestCase):
    def test_training_options_are_bounded(self):
        options = dict(epochs=1, batch_size=2, rate=0.001, l2=0.0, seed=1, label_mix=0.5, k=1.0)
        train.validate_options(**options)
        for field, value in (("epochs", 0), ("rate", float("nan")), ("seed", -1), ("batch_size", 0),
                             ("label_mix", 2), ("threads", 0)):
            with self.subTest(field=field), self.assertRaises(ValueError):
                train.validate_options(**(options | {field: value}))


@unittest.skipUnless(HELPER is not None, "set JAKGRO_NNUE_DATA to a freshly built nnue-data for end-to-end tests")
class RustPipelineTests(unittest.TestCase):
    def test_changed_tool_sources_refuse_training_or_publication(self):
        original = train.tool_source_hashes()
        changed = original | {"train_nnue.py": "0" * 64}
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            training, development = self.make_inputs(root)
            data.prepare(HELPER, training, development, root / "prepared")
            with patch.object(train, "tool_source_hashes", return_value=changed):
                with self.assertRaisesRegex(ValueError, "changed since import"):
                    train.train(root / "prepared", HELPER, root / "early", epochs=1)
            self.assertFalse((root / "early").exists())
            with patch.object(train, "tool_source_hashes", side_effect=[original, changed]), contextlib.redirect_stdout(io.StringIO()):
                with self.assertRaisesRegex(ValueError, "changed during training"):
                    train.train(root / "prepared", HELPER, root / "late", epochs=1, batch_size=2, l2=0)
            self.assertFalse((root / "late").exists())

    def make_inputs(self, directory):
        training, development = directory / "training.txt", directory / "development.txt"
        training.write_text("".join(row.fen + ";1;150\n" for row in (SAMPLE, sample_with_pawn(3), sample_with_pawn(4))))
        development.write_text("".join(row.fen + ";1;150\n" for row in (sample_with_pawn(5), sample_with_pawn(6))))
        return training, development

    def test_rust_preparation_binds_features_hashes_and_rejects_bad_data(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            training, development = self.make_inputs(root)
            manifest = data.prepare(HELPER, training, development, root / "prepared")
            loaded, train_rows, dev_rows = data.load_dataset(root / "prepared", HELPER)
            self.assertEqual(manifest, loaded)
            self.assertEqual(train_rows[0], SAMPLE)
            self.assertEqual((len(train_rows), len(dev_rows)), (3, 2))
            with self.assertRaisesRegex(ValueError, "already exists"):
                data.prepare(HELPER, training, development, root / "prepared")
            original = (root / "prepared/training.tsv").read_bytes()
            (root / "prepared/training.tsv").write_bytes(original + b"x")
            with self.assertRaisesRegex(ValueError, "hash mismatch"):
                data.load_dataset(root / "prepared", HELPER)
            training.write_text(FEN + ";NaN\n")
            with self.assertRaisesRegex(ValueError, "preparation failed"):
                data.prepare(HELPER, training, development, root / "bad")
            self.assertFalse((root / "bad").exists())

    def test_preparation_rejects_feature_leakage_even_with_different_clocks(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            training, development = self.make_inputs(root)
            development.write_text(FEN.replace("0 1", "2 9") + ";1;150\n")
            with self.assertRaisesRegex(ValueError, "overlap"):
                data.prepare(HELPER, training, development, root / "bad", True)
            self.assertFalse((root / "bad").exists())

    def test_python_export_matches_rust_accumulators_and_output(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            training, development = self.make_inputs(root)
            data.prepare(HELPER, training, development, root / "prepared")
            _, rows, more = data.load_dataset(root / "prepared", HELPER)
            network = zero_network(-16321)
            inputs = array("h", network.input_weights)
            for feature in SAMPLE.white:
                inputs[feature * fmt.HIDDEN] = 97
            network = replace(network, input_weights=inputs, output_weights=(640,) + (0,) * 255)
            model = root / "network.nnue"
            model.write_bytes(network.to_bytes())
            result = train.verify_rust(HELPER, model, rows + more, root)
            self.assertTrue(result["passed"])
            self.assertEqual(result["positions"], 5)
            corrupted = bytearray(model.read_bytes())
            corrupted[-1] ^= 1
            model.write_bytes(corrupted)
            scored = subprocess.run([str(HELPER), "score", str(model), "-"], input=FEN + "\n", text=True, capture_output=True)
            self.assertNotEqual(scored.returncode, 0)
            self.assertIn("checksum", scored.stderr)

    def test_real_training_is_deterministic_and_exports_only_after_parity(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            training, development = self.make_inputs(root)
            data.prepare(HELPER, training, development, root / "prepared")
            reports = []
            for name in ("first", "second"):
                with contextlib.redirect_stdout(io.StringIO()):
                    report = train.train(root / "prepared", HELPER, root / name, epochs=3,
                                         batch_size=2, rate=0.001, l2=0.0, seed=75)
                self.assertTrue(report["training_completed"])
                self.assertEqual(report["steps"], 6)
                self.assertLess(report["selected_metrics"]["integer_training"]["label_mse"],
                                report["initial_integer_training"]["label_mse"])
                self.assertEqual(report["parity_positions"], 2)
                reports.append(report)
            self.assertEqual((root / "first/network.nnue").read_bytes(), (root / "second/network.nnue").read_bytes())
            self.assertEqual(reports[0]["history"], reports[1]["history"])
            with self.assertRaisesRegex(ValueError, "epochs"):
                train.train(root / "prepared", HELPER, root / "zero", epochs=0)
            self.assertFalse((root / "zero").exists())

    def test_failed_parity_does_not_publish_an_output(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            training, development = self.make_inputs(root)
            data.prepare(HELPER, training, development, root / "prepared")
            with patch.object(train, "verify_rust", side_effect=ValueError("injected parity failure")), contextlib.redirect_stdout(io.StringIO()):
                with self.assertRaisesRegex(ValueError, "parity failure"):
                    train.train(root / "prepared", HELPER, root / "bad", epochs=1, batch_size=2, l2=0)
            self.assertFalse((root / "bad").exists())


if __name__ == "__main__":
    unittest.main()
