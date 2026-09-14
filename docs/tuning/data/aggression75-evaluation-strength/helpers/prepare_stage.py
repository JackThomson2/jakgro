#!/usr/bin/env python3
"""Reconstruct one recorded source stage in a new directory outside the checkout."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

sys.dont_write_bytecode = True
from archive_support import ARCHIVE, ROOT, digest, read_json, require, source_digests, stages


def prepare(stage: str, destination: Path) -> None:
    destination = destination.resolve()
    require(destination != ROOT and ROOT not in destination.parents,
            "destination must be outside the repository")
    require(not destination.exists(), "destination already exists; choose a new directory")
    provenance = read_json(ARCHIVE / "provenance.json")
    source = stages()[stage]
    require(source_digests(source) == provenance["source_stages"][stage], "source identity mismatch")
    for name, expected in provenance["frozen_inputs"].items():
        payload = (ARCHIVE / "inputs" / name).read_bytes()
        require(digest(payload) == expected, f"input identity mismatch: {name}")
        require(name not in source or source[name] == payload, f"conflicting input: {name}")
        source[name] = payload
    for record in provenance["corpora"].values():
        payload = (ROOT / record["path"]).read_bytes()
        require(digest(payload) == record["sha256"], f"corpus identity: {record['path']}")
        source[record["path"]] = payload
    destination.mkdir(parents=True)
    for name, payload in source.items():
        path = destination / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(payload)
    print(f"Prepared {stage}: {len(source)} verified files in {destination}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--stage", required=True,
                        choices=("base", "raw-mobility-1", "safe-mobility-1",
                                 "safe-prior", "memo-8192", "memo-tagged"))
    parser.add_argument("--out", required=True, type=Path)
    arguments = parser.parse_args()
    prepare(arguments.stage, arguments.out)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError) as error:
        print(f"prepare_stage: {error}", file=sys.stderr)
        raise SystemExit(1)
