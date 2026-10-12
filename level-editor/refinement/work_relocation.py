"""Explicit relocation of operational references without rewriting historical evidence."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
from typing import Any

SHA = re.compile(r"[0-9a-f]{64}\Z")


def file_sha256(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def discover_repo_root(start: Path | str) -> Path:
    start = Path(start).resolve()
    if start.is_file():
        start = start.parent
    for candidate in (start, *start.parents):
        if ((candidate / "AGENTS.md").is_file()
                and (candidate / "Cargo.toml").is_file()
                and (candidate / "level-editor").is_dir()):
            return candidate
    raise ValueError(f"Repository markers not found above {start}")


def _path(value: str) -> str:
    if not isinstance(value, str) or not value or "\\" in value:
        raise ValueError(f"Expected explicit POSIX path: {value!r}")
    if ".." in PurePosixPath(value).parts:
        raise ValueError(f"Parent traversal is not a relocation path: {value}")
    return str(PurePosixPath(value))


def _hash(value: str) -> str:
    if not isinstance(value, str) or not SHA.fullmatch(value):
        raise ValueError(f"Invalid SHA-256: {value!r}")
    return value


class RelocationMap:
    """Resolve only declared mappings; no filesystem search or legacy fallback."""

    def __init__(self, manifest: dict, repo_root: Path | str):
        if manifest.get("version") != 1:
            raise ValueError("Unsupported relocation manifest version")
        self.root = Path(repo_root).resolve()
        self.prefixes: dict[str, str] = {}
        self.historical: dict[tuple[str, str], str] = {}
        self.operational: dict[tuple[str, str], tuple[str, str]] = {}
        self.changes: list[dict] = []
        for row in manifest.get("path_mappings", []):
            old, new = _path(row["old_prefix"]), _path(row["new_prefix"])
            if old in self.prefixes or old == new:
                raise ValueError(f"Duplicate or identity prefix: {old}")
            self.prefixes[old] = new
        for row in manifest.get("historical_files", []):
            key = (_path(row["old_path"]), _hash(row["sha256"]))
            if key in self.historical:
                raise ValueError(f"Duplicate historical mapping: {key}")
            self.historical[key] = _path(row["preserved_path"])
        for row in manifest.get("operational_files", []):
            key = (_path(row["old_path"]), _hash(row["old_sha256"]))
            if key in self.operational:
                raise ValueError(f"Duplicate operational mapping: {key}")
            self.operational[key] = (_path(row["new_path"]), _hash(row["new_sha256"]))
        # Two operational revisions of one old path need hash context; plain paths
        # must not silently choose one of them.

    def absolute(self, value: str) -> Path:
        p = Path(_path(value))
        return p if p.is_absolute() else self.root / p

    def _verify(self, value: str, expected: str) -> Path:
        p = self.absolute(value)
        if any(part.is_symlink() for part in (p, *p.parents)):
            raise ValueError(f"Relocation target must not be a compatibility symlink: {p}")
        if not p.is_file() or file_sha256(p) != _hash(expected):
            raise ValueError(f"Missing or changed relocation target: {p}")
        return p

    def translate_path(self, value: str) -> str:
        """Translate a whole path; unmapped strings remain untouched."""
        if not isinstance(value, str):
            raise TypeError("Path must be a string")
        for old in sorted(self.prefixes, key=len, reverse=True):
            if value == old or value.startswith(old + "/"):
                _path(value)  # Reject traversal instead of normalizing it away.
                return self.prefixes[old] + value[len(old):]
        return value

    def resolve_historical(self, old_path: str, expected: str) -> Path:
        key = (_path(old_path), _hash(expected))
        if key not in self.historical:
            raise ValueError(f"No explicit historical file mapping: {key}")
        return self._verify(self.historical[key], expected)

    def read_historical_json(self, old_path: str, expected: str, *, translate: bool = False) -> Any:
        value = json.loads(self.resolve_historical(old_path, expected).read_bytes())
        return self.translate_json(value) if translate else value

    def verify(self) -> dict:
        for (old, digest) in self.historical:
            self.resolve_historical(old, digest)
        for key, (new, digest) in self.operational.items():
            self.resolve_historical(*key)
            self._verify(new, digest)
        return {"historical_files": len(self.historical),
                "operational_files": len(self.operational),
                "path_mappings": len(self.prefixes)}

    def _pinned(self, old: str, digest: str) -> tuple[str, str]:
        key = (_path(old), _hash(digest))
        if key in self.operational:
            self.resolve_historical(*key)
            new, new_hash = self.operational[key]
            self._verify(new, new_hash)
        else:
            new, new_hash = self.translate_path(old), digest
            if new == old:
                return old, digest
            # A plain relocation must retain exactly the approved bytes. A
            # semantically edited target needs its own declared operational copy.
            self._verify(new, digest)
        self.changes.append({"old_path": old, "old_sha256": digest,
                             "new_path": new, "new_sha256": new_hash})
        return new, new_hash

    def translate_json(self, value: Any) -> Any:
        if isinstance(value, list):
            return [self.translate_json(v) for v in value]
        if not isinstance(value, dict):
            return self.translate_path(value) if isinstance(value, str) else value
        output: dict = {}
        paired: set[str] = set()
        # Common structured manifest pins. Other formats are left explicit:
        # callers must convert them or add a supported pair, never infer hashes.
        pairs = [(k, "sha256") for k in ("path", "file")]
        pairs += [(k, k + "_sha256") for k in value]
        pairs += [(k, k[:-5] + "_sha256") for k in value if k.endswith("_path")]
        for path_key, hash_key in pairs:
            if (path_key in paired or hash_key in paired or path_key == hash_key
                    or not isinstance(value.get(path_key), str)
                    or not isinstance(value.get(hash_key), str)
                    or not SHA.fullmatch(value[hash_key])):
                continue
            old = value[path_key]
            if self.translate_path(old) == old and (old, value[hash_key]) not in self.operational:
                continue
            output[path_key], output[hash_key] = self._pinned(old, value[hash_key])
            paired.update((path_key, hash_key))
        for key, item in value.items():
            if key in paired:
                continue
            new_key = self.translate_path(key)
            if new_key in output:
                raise ValueError(f"Relocation collides at JSON key: {new_key}")
            if isinstance(item, str) and SHA.fullmatch(item):
                if new_key != key or (key, item) in self.operational:
                    new_key, translated = self._pinned(key, item)
                else:
                    translated = item
            else:
                translated = self.translate_json(item)
            if new_key in output:
                raise ValueError(f"Relocation collides at JSON key: {new_key}")
            output[new_key] = translated
        return output


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["verify", "translate"])
    parser.add_argument("mapping", type=Path)
    parser.add_argument("input", nargs="?", type=Path)
    parser.add_argument("output", nargs="?", type=Path)
    parser.add_argument("--repo-root", type=Path)
    parser.add_argument("--input-sha256")
    args = parser.parse_args()
    root = args.repo_root or discover_repo_root(args.mapping)
    relocation = RelocationMap(json.loads(args.mapping.read_text()), root)
    verified = relocation.verify()
    if args.command == "verify":
        print(json.dumps({"status": "VERIFIED_EXPLICIT_RELOCATION", **verified}))
        return
    if args.input is None or args.output is None:
        parser.error("translate requires INPUT and new OUTPUT")
    raw = args.input.read_bytes()
    before = hashlib.sha256(raw).hexdigest()
    if args.input_sha256 and before != _hash(args.input_sha256):
        raise ValueError("Historical input hash differs")
    if args.output.exists() or args.input.resolve() == args.output.resolve():
        raise ValueError("Operational output must be new; historical input is immutable")
    translated = relocation.translate_json(json.loads(raw))
    data = (json.dumps(translated, indent=2, ensure_ascii=False) + "\n").encode()
    receipt = args.output.with_name(args.output.name + ".relocation.json")
    if receipt.exists():
        raise ValueError("Relocation receipt already exists")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("xb") as stream:
        stream.write(data)
    receipt.write_text(json.dumps({"version": 1, "historical_input": str(args.input),
        "historical_sha256": before, "operational_output": str(args.output),
        "operational_sha256": hashlib.sha256(data).hexdigest(),
        "mapping_sha256": file_sha256(args.mapping), "pin_changes": relocation.changes}, indent=2) + "\n")
    print(json.dumps({"status": "NEW_OPERATIONAL_MANIFEST_WRITTEN", "output": str(args.output),
                      "receipt": str(receipt), "pin_changes": len(relocation.changes)}))


if __name__ == "__main__":
    main()
