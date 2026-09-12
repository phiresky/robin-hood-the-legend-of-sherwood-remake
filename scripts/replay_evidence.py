"""Checksum and sealed-bundle admission; no ledger or scheduling state."""
from __future__ import annotations
import os
import re
import hashlib
from pathlib import Path

def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()

def verify_manifest(result: Path, required: set[str]) -> str:
    """Admit a sealed result only when every consumed file is covered."""
    return _verify_checksum_file(result.resolve(), "MANIFEST.sha256", required)


def _safe_manifest_path(root: Path, relative: str) -> Path:
    candidate_relative = Path(relative)
    if (not relative or candidate_relative.as_posix() != relative
            or candidate_relative.is_absolute()
            or ".." in candidate_relative.parts):
        raise ValueError(f"unsafe manifest path: {relative}")
    candidate = root / candidate_relative
    if candidate.is_symlink() or not candidate.is_file():
        raise ValueError(f"manifest entry is not a regular file: {relative}")
    resolved = candidate.resolve(strict=True)
    try:
        resolved.relative_to(root)
    except ValueError as error:
        raise ValueError(f"manifest entry escapes audit root: {relative}") from error
    return candidate


def verify_sealed_manifest(root: Path, expected: set[str]) -> str:
    manifest = root / "MANIFEST.sha256"
    if manifest.is_symlink() or not manifest.is_file():
        raise ValueError(f"sealed audit has no regular MANIFEST.sha256: {root}")
    observed: set[str] = set()
    for number, line in enumerate(manifest.read_text(errors="strict").splitlines(), 1):
        match = re.fullmatch(r"([0-9a-fA-F]{64}) [ *](.+)", line)
        if not match:
            raise ValueError(f"{manifest}:{number}: malformed checksum")
        digest, relative = match.groups()
        if relative in observed:
            raise ValueError(f"{manifest}:{number}: duplicate path {relative}")
        candidate = _safe_manifest_path(root, relative)
        if sha256_file(candidate) != digest.lower():
            raise ValueError(f"{manifest}:{number}: checksum mismatch for {relative}")
        observed.add(relative)
    if observed != expected:
        missing = sorted(expected - observed)
        extra = sorted(observed - expected)
        raise ValueError(
            f"{manifest}: sealed membership mismatch; missing={missing}, extra={extra}"
        )
    return sha256_file(manifest)


def _parse_zero_sha256_manifest(path: Path) -> list[tuple[str, str]]:
    entries: list[tuple[str, str]] = []
    seen: set[str] = set()
    raw_entries = path.read_bytes().split(b"\0")
    if not raw_entries or raw_entries[-1] != b"":
        raise ValueError(f"{path}: checksum manifest is not NUL terminated")
    for number, raw in enumerate(raw_entries[:-1], 1):
        if (len(raw) < 67 or raw[64:66] not in (b"  ", b" *")
                or not re.fullmatch(rb"[0-9a-fA-F]{64}", raw[:64])):
            raise ValueError(f"{path}:{number}: malformed NUL checksum entry")
        try:
            name = os.fsdecode(raw[66:])
        except UnicodeError as error:
            raise ValueError(f"{path}:{number}: invalid path encoding") from error
        if not name or name in seen:
            raise ValueError(f"{path}:{number}: empty or duplicate path")
        seen.add(name)
        entries.append((raw[:64].decode().lower(), name))
    return entries


def _parse_zero_paths(path: Path) -> list[str]:
    raw_entries = path.read_bytes().split(b"\0")
    if not raw_entries or raw_entries[-1] != b"":
        raise ValueError(f"{path}: path snapshot is not NUL terminated")
    paths = [os.fsdecode(raw) for raw in raw_entries[:-1]]
    if any(not value for value in paths) or len(paths) != len(set(paths)):
        raise ValueError(f"{path}: empty or duplicate path")
    return paths


def _verify_checksum_file(
    root: Path, manifest_name: str, required: set[str] | None = None
) -> str:
    manifest = _safe_manifest_path(root, manifest_name)
    seen: set[str] = set()
    for number, line in enumerate(manifest.read_text(errors="strict").splitlines(), 1):
        match = re.fullmatch(r"([0-9a-fA-F]{64}) [ *](.+)", line)
        if not match:
            raise ValueError(f"{manifest}:{number}: malformed checksum")
        expected, relative = match.groups()
        if relative in seen:
            raise ValueError(f"{manifest}:{number}: duplicate path {relative}")
        candidate = _safe_manifest_path(root, relative)
        if sha256_file(candidate) != expected.lower():
            raise ValueError(f"{manifest}:{number}: checksum mismatch for {relative}")
        seen.add(relative)
    if not seen:
        raise ValueError(f"{manifest}: empty checksum manifest")
    if required and not required.issubset(seen):
        raise ValueError(f"{manifest}: missing required bundle members {sorted(required - seen)}")
    return sha256_file(manifest)

