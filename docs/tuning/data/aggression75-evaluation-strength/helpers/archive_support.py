"""Read-only reconstruction of the evaluation trial's source stages."""

from __future__ import annotations

import hashlib
import json
import re
from pathlib import Path

ARCHIVE = Path(__file__).resolve().parents[1]
ROOT = ARCHIVE.parents[3]


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def digest(payload: bytes) -> str:
    return hashlib.sha256(payload).hexdigest()


def read_json(path: Path):
    return json.loads(path.read_text(encoding="utf-8"))


def patch_sources(sources: dict[str, bytes], patch: str, reverse: bool = False) -> dict[str, bytes]:
    """Apply ordinary unified source hunks in memory, checking every context byte."""
    result = dict(sources)
    sections = re.split(r"(?m)(?=^diff --git )", patch)
    for section in sections:
        if not section.startswith("diff --git "):
            continue
        lines = section.splitlines(keepends=True)
        old_header = next(line for line in lines if line.startswith("--- ")).strip()[4:]
        new_header = next(line for line in lines if line.startswith("+++ ")).strip()[4:]
        old = None if old_header == "/dev/null" else old_header.removeprefix("a/")
        new = None if new_header == "/dev/null" else new_header.removeprefix("b/")
        if reverse:
            old, new = new, old
        require(old is not None or new is not None, "patch has no source path")
        for path in (old, new):
            if path is not None:
                require(not Path(path).is_absolute() and ".." not in Path(path).parts, "unsafe patch path")
        require(old is None or old in result, f"missing patch input: {old}")
        require(old is not None or new not in result, f"patch creation would overwrite {new}")
        original = result[old].decode("utf-8").splitlines(keepends=True) if old else []
        output: list[str] = []
        consumed = 0
        hunks = [index for index, line in enumerate(lines) if line.startswith("@@ ")]
        require(bool(hunks), f"no source hunks in {old or new}")
        for number, start in enumerate(hunks):
            match = re.match(r"@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@", lines[start])
            require(match is not None, "invalid hunk header")
            old_start, old_count, new_start, new_count = match.groups()
            first, count = (new_start, new_count) if reverse else (old_start, old_count)
            target = max(0, int(first) - 1)
            require(consumed <= target <= len(original), "overlapping or out-of-range hunk")
            output.extend(original[consumed:target])
            consumed = target
            before = consumed
            end = hunks[number + 1] if number + 1 < len(hunks) else len(lines)
            for line in lines[start + 1:end]:
                if line.startswith("\\ No newline"):
                    raise ValueError("source patches must preserve final newlines")
                if not line or line[0] not in " +-":
                    continue
                operation = line[0]
                if reverse:
                    operation = {"+": "-", "-": "+", " ": " "}[operation]
                content = line[1:]
                if operation in " -":
                    require(consumed < len(original) and original[consumed] == content,
                            f"hunk context mismatch in {old}: line {consumed + 1}")
                    consumed += 1
                if operation in " +":
                    output.append(content)
            require(consumed - before == int(count or 1), "hunk source count mismatch")
        output.extend(original[consumed:])
        if old is not None:
            del result[old]
        if new is not None:
            result[new] = "".join(output).encode("utf-8")
        else:
            require(not output, f"deletion left content in {old}")
    return result


def stages(root: Path = ROOT, archive: Path = ARCHIVE) -> dict[str, dict[str, bytes]]:
    provenance = read_json(archive / "provenance.json")
    current = {name: (root / name).read_bytes() for name in provenance["source_files"]}
    final_patch = (archive / "data/memo-tagged.patch").read_text(encoding="utf-8")
    base = patch_sources(current, final_patch, reverse=True)
    require(patch_sources(base, final_patch) == current, "final patch round trip")
    raw = patch_sources(base, (archive / "data/raw-mobility-1.patch").read_text(encoding="utf-8"))
    safe = patch_sources(base, (archive / "data/safe-mobility-1.patch").read_text(encoding="utf-8"))
    safe_prior = dict(base)
    safe_prior["src/engine/evaluation/features.rs"] = safe["src/engine/evaluation/features.rs"]
    untagged = dict(current)
    untagged["src/engine/evaluation/memo.rs"] = (archive / "data/memo-8192-source.rs").read_bytes()
    return {"base": base, "raw-mobility-1": raw, "safe-mobility-1": safe,
            "safe-prior": safe_prior, "memo-8192": untagged, "memo-tagged": current}


def source_digests(source: dict[str, bytes]) -> dict[str, str]:
    return {name: digest(payload) for name, payload in sorted(source.items())}
