"""Checksum and sealed-bundle admission; no ledger or scheduling state."""
from __future__ import annotations
import os
import re
import hashlib
from dataclasses import dataclass
from pathlib import Path
from types import MappingProxyType
from typing import Mapping

# Admission limits, not producer limits. Refuse oversized evidence before parsing.
MAX_MANIFEST_BYTES = 16 * 1024 * 1024
MAX_MEMBER_BYTES = 256 * 1024 * 1024
MAX_SNAPSHOT_BYTES = 512 * 1024 * 1024


@dataclass(frozen=True)
class EvidenceSnapshot:
    manifest_sha256: str | None
    files: Mapping[str, bytes]
    checksums: Mapping[str, str]


def _read_bounded(path: Path, limit: int) -> bytes:
    with path.open("rb") as handle:
        data = handle.read(limit + 1)
    if len(data) > limit:
        raise ValueError(f"{path}: evidence exceeds {limit} byte admission limit")
    return data


def read_evidence_snapshot(
    root: Path, consumed: set[str], *, historical: bool = False,
    exact: bool = False, manifest_name: str = "MANIFEST.sha256",
    manifest_bytes: bytes | None = None,
) -> EvidenceSnapshot:
    """Hash and retain the identical bytes consumers will parse.

    Historical recovery may consume unsealed members, but never upgrades them
    to attested evidence. Unconsumed members are streamed, not retained.
    """
    root = root.resolve()
    manifest = root / manifest_name
    checksums: dict[str, str] = {}
    files: dict[str, bytes] = {}
    remaining = MAX_SNAPSHOT_BYTES
    raw = manifest_bytes
    if raw is None and (not historical or manifest.exists() or manifest.is_symlink()):
        raw = _read_bounded(_safe_manifest_path(root, manifest_name), MAX_MANIFEST_BYTES)
    if raw is not None:
        if len(raw) > MAX_MANIFEST_BYTES:
            raise ValueError(f"{manifest}: manifest exceeds admission limit")
        for number, line in enumerate(raw.decode().splitlines(), 1):
            match = re.fullmatch(r"([0-9a-fA-F]{64}) [ *](.+)", line)
            if not match:
                raise ValueError(f"{manifest}:{number}: malformed checksum")
            expected, relative = match.groups()
            if relative in checksums:
                raise ValueError(f"{manifest}:{number}: duplicate path {relative}")
            candidate = _safe_manifest_path(root, relative)
            if relative in consumed:
                data = _read_bounded(candidate, min(MAX_MEMBER_BYTES, remaining))
                remaining -= len(data)
                files[relative] = data
                actual = hashlib.sha256(data).hexdigest()
            else:
                actual = sha256_file(candidate)
            if actual != expected.lower():
                raise ValueError(f"{manifest}:{number}: checksum mismatch for {relative}")
            checksums[relative] = actual
        if not checksums:
            raise ValueError(f"{manifest}: empty checksum manifest")
    missing = consumed - checksums.keys()
    if not historical and missing:
        raise ValueError(f"{manifest}: missing required bundle members {sorted(missing)}")
    if exact and checksums.keys() != consumed:
        raise ValueError(f"{manifest}: sealed membership mismatch")
    if historical:
        for relative in sorted(missing):
            data = _read_bounded(_safe_manifest_path(root, relative),
                                 min(MAX_MEMBER_BYTES, remaining))
            remaining -= len(data)
            files[relative] = data
    return EvidenceSnapshot(
        hashlib.sha256(raw).hexdigest() if raw is not None else None,
        MappingProxyType(files), MappingProxyType(checksums),
    )


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


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


def _parse_zero_sha256_manifest(path: Path, data: bytes) -> list[tuple[str, str]]:
    entries: list[tuple[str, str]] = []
    seen: set[str] = set()
    raw_entries = data.split(b"\0")
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


def _parse_zero_paths(path: Path, data: bytes) -> list[str]:
    raw_entries = data.split(b"\0")
    if not raw_entries or raw_entries[-1] != b"":
        raise ValueError(f"{path}: path snapshot is not NUL terminated")
    paths = [os.fsdecode(raw) for raw in raw_entries[:-1]]
    if any(not value for value in paths) or len(paths) != len(set(paths)):
        raise ValueError(f"{path}: empty or duplicate path")
    return paths
