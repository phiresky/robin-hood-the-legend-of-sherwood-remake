#!/usr/bin/env python3
"""Update permanent exact-EOF manifests without ever removing an entry."""

from __future__ import annotations

import json
import os
from pathlib import Path
import tempfile


ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "docs" / "PARITY_EOF_LEDGERS"
EOF_MARKER = "parity trace matched every recorded frame"


def load_existing(path: Path) -> set[str]:
    if not path.exists():
        return set()
    return {line for raw in path.read_text().splitlines() if (line := raw.strip())}


def atomic_lines(path: Path, values: set[str]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(fd, "w") as handle:
            for value in sorted(values):
                handle.write(f"{value}\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def exact_keys(audit_roots: list[Path], allowed: set[str] | None = None) -> set[str]:
    result: set[str] = set()
    for audit in audit_roots:
        status_dir = audit / "status"
        log_dir = audit / "logs"
        if not status_dir.is_dir() or not log_dir.is_dir():
            continue
        for status in status_dir.glob("*.status"):
            key = status.name.removesuffix(".status")
            if allowed is not None and key not in allowed:
                continue
            if status.read_text().strip() != "0":
                continue
            log = log_dir / f"{key}.log"
            if not log.is_file():
                continue
            marker_count = sum(line == EOF_MARKER for line in log.read_text(errors="replace").splitlines())
            if marker_count == 1:
                result.add(key)
    return result


def update(name: str, discovered: set[str]) -> set[str]:
    path = OUTPUT / f"{name}.snapshot"
    combined = load_existing(path) | discovered
    atomic_lines(path, combined)
    return combined


def main() -> None:
    cache_audits = Path("/home/phire/.cache/sccache/robinhood-parity-audits")

    original_root = cache_audits / "nonseed-final-b74d55ebf-20260818"
    original = update("original", exact_keys([original_root]))

    one_root = cache_audits / "seed1000000-final-b74d55ebf-20260818"
    one_sources = [one_root, *sorted(one_root.glob("rerun-*"))]
    curated = {
        line.strip().replace("/", "__")
        for line in (ROOT / "parity-save-replays" / "seed1000000-final-20260818.snapshot")
        .read_text()
        .splitlines()
        if line.strip()
    }
    one_m = update("seed1000000", exact_keys(one_sources, curated))

    local_audits = ROOT / "parity-save-replays" / "audits"
    two_sources = [path for path in sorted(local_audits.iterdir()) if path.is_dir()]
    two_discovered = {
        key
        for key in exact_keys(two_sources)
        if "__schema16-seed2000000-20260820__" in key
    }
    two_m = update("seed2000000", two_discovered)

    temporary_root = ROOT / ".codex-tmp"
    interactive_sources = [
        path.parent
        for path in temporary_root.glob("*/status")
        if path.is_dir()
    ]
    interactive_discovered = {
        key for key in exact_keys(interactive_sources) if "__interactive__" in key
    }
    interactive = update("interactive", interactive_discovered)

    captured_two_m = len(
        list(
            (ROOT / "parity-save-replays/60s-random-input/schema16-seed2000000-20260820/traces")
            .rglob("*.jsonl.zst")
        )
    )
    summary = {
        "original": {"eof": len(original), "planned": 4800},
        "seed1000000": {"eof": len(one_m), "planned": 2430},
        "seed2000000": {"eof": len(two_m), "planned": 9720, "captured": captured_two_m},
        "interactive": {"eof": len(interactive), "planned": 12},
    }
    summary_path = OUTPUT / "summary.json"
    fd, temporary = tempfile.mkstemp(prefix=".summary.", dir=OUTPUT)
    try:
        with os.fdopen(fd, "w") as handle:
            json.dump(summary, handle, indent=2, sort_keys=True)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, summary_path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)
    print(json.dumps(summary, sort_keys=True))


if __name__ == "__main__":
    main()
