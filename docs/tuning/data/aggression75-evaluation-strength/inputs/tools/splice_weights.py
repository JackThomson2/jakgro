#!/usr/bin/env python3
"""Splice a `tune fit` weight file into the evaluation sources.

The fitter writes compilable Rust under banners naming the file each block
belongs to. Until now that text was pasted by hand, and the fourth series
records what one mis-rebuilt paste cost: a second fit whose `--hold` pinned
blocks to the first fit's values rather than the published ones, because
"published" is whatever is compiled in. This tool replaces each declaration
the fit names, in place, and refuses anything it cannot match exactly, so a
refit is one command and the tree it leaves is the one the fit describes.

For an array the target's own type is kept, so a length written as a named
constant stays a named constant; only the entries are replaced, whether the
target lists them or writes a zero repeat. A scalar is one line and a table
is one block, both replaced whole.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

BANNER = re.compile(r"^// ---- (?P<path>\S+) ----$")
SCALAR = re.compile(r"^const (?P<name>[A-Z0-9_]+): ScorePair = ScorePair::new\(-?\d+, -?\d+\);$")
ARRAY_START = re.compile(r"^const (?P<name>[A-Z0-9_]+): \[ScorePair; (?P<len>\d+)\] = \[$")
TABLE_START = re.compile(r"^static (?P<name>[A-Z0-9_]+): Table = Table \{$")


class SpliceError(Exception):
    """A declaration in the fit could not be matched to exactly one target."""


def parse_fit(text: str) -> dict[str, list[tuple[str, str, list[str]]]]:
    """Return, per target path, the (kind, name, lines) declarations in order."""
    declarations: dict[str, list[tuple[str, str, list[str]]]] = {}
    path: str | None = None
    lines = text.splitlines()
    index = 0
    while index < len(lines):
        line = lines[index]
        banner = BANNER.match(line)
        if banner:
            path = str(banner.group("path"))
            declarations.setdefault(path, [])
            index += 1
            continue
        if not line.strip() or line.startswith("//"):
            index += 1
            continue
        if path is None:
            raise SpliceError(f"line {index + 1} precedes any file banner: {line!r}")
        scalar = SCALAR.match(line)
        if scalar:
            declarations[path].append(("scalar", scalar.group("name"), [line]))
            index += 1
            continue
        array = ARRAY_START.match(line)
        if array:
            body: list[str] = []
            index += 1
            while index < len(lines) and lines[index] != "];":
                body.append(lines[index])
                index += 1
            if index >= len(lines):
                raise SpliceError(f"array {array.group('name')} is not terminated")
            if len(body) != int(array.group("len")):
                raise SpliceError(
                    f"array {array.group('name')} declares {array.group('len')} entries "
                    f"but lists {len(body)}"
                )
            declarations[path].append(("array", array.group("name"), body))
            index += 1
            continue
        table = TABLE_START.match(line)
        if table:
            block = [line]
            index += 1
            while index < len(lines) and lines[index] != "};":
                block.append(lines[index])
                index += 1
            if index >= len(lines):
                raise SpliceError(f"table {table.group('name')} is not terminated")
            block.append("};")
            declarations[path].append(("table", table.group("name"), block))
            index += 1
            continue
        raise SpliceError(f"line {index + 1} is not a declaration the fitter emits: {line!r}")
    return declarations


def _find_declaration(lines: list[str], kind: str, name: str) -> tuple[int, int]:
    """Return the [start, end) line range of the named declaration."""
    keyword = "static" if kind == "table" else "const"
    prefix = f"{keyword} {name}: "
    starts = [number for number, line in enumerate(lines) if line.startswith(prefix)]
    if not starts:
        raise SpliceError(f"{name} is not declared in the target")
    if len(starts) > 1:
        raise SpliceError(f"{name} is declared {len(starts)} times in the target")
    start = starts[0]
    if kind == "scalar":
        return start, start + 1
    # An array in the target may be the fitter's own multi-line form, a
    # one-line repeat such as `[ScorePair::new(0, 0); 6]`, or that repeat
    # wrapped onto a second line; all end at the first line closing with `];`.
    # A table ends at its closing brace.
    end = start
    while end < len(lines):
        closed = lines[end].rstrip().endswith("];") if kind == "array" else lines[end] == "};"
        if closed:
            return start, end + 1
        end += 1
    raise SpliceError(f"{name} is not terminated in the target")


def splice(source: str, declarations: list[tuple[str, str, list[str]]]) -> tuple[str, list[str]]:
    """Return the source with every declaration replaced, and the names replaced."""
    lines = source.splitlines()
    replaced: list[str] = []
    for kind, name, body in declarations:
        start, end = _find_declaration(lines, kind, name)
        if kind == "array":
            header = lines[start].split(" =", 1)[0] + " = ["
            replacement = [header, *body, "];"]
        else:
            replacement = body
        lines[start:end] = replacement
        replaced.append(name)
    return "\n".join(lines) + "\n", replaced


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=(__doc__ or "").split("\n", 1)[0])
    parser.add_argument("fit", type=Path, help="weight file written by `tune fit --out`")
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(__file__).resolve().parent.parent,
        help="repository root the fit's banners are relative to",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="report what would be replaced without writing",
    )
    arguments = parser.parse_args(argv)

    try:
        declarations = parse_fit(arguments.fit.read_text())
        for relative, blocks in declarations.items():
            target = arguments.root / relative
            if not target.is_file():
                raise SpliceError(f"{relative} does not exist under {arguments.root}")
            spliced, replaced = splice(target.read_text(), blocks)
            for name in replaced:
                print(f"{relative}: {name}")
            if not arguments.dry_run:
                target.write_text(spliced)
    except SpliceError as error:
        print(f"splice_weights: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
