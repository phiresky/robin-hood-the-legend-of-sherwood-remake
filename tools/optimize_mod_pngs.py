#!/usr/bin/env python3
"""Run oxipng over mod assets without breaking duplicate-frame hard links."""

from __future__ import annotations

import argparse
from collections import defaultdict
import os
from pathlib import Path
import subprocess


def physical_size(paths: list[Path]) -> int:
    seen: set[tuple[int, int]] = set()
    total = 0
    for path in paths:
        stat = path.stat()
        key = (stat.st_dev, stat.st_ino)
        if key not in seen:
            seen.add(key)
            total += stat.st_size
    return total


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("roots", nargs="+", type=Path)
    parser.add_argument("--level", type=int, default=2, choices=range(0, 7))
    parser.add_argument("--batch-size", type=int, default=128)
    args = parser.parse_args()

    pngs = sorted(
        path
        for root in args.roots
        for path in root.rglob("*.png")
        if path.is_file()
    )
    groups: dict[tuple[int, int], list[Path]] = defaultdict(list)
    for path in pngs:
        stat = path.stat()
        groups[(stat.st_dev, stat.st_ino)].append(path)
    canonical = [paths[0] for paths in groups.values()]
    before = physical_size(pngs)
    print(
        f"Optimizing {len(pngs)} PNG paths ({len(canonical)} unique payloads, "
        f"{before / 1024 / 1024:.1f} MiB)"
    )

    for start in range(0, len(canonical), args.batch_size):
        batch = canonical[start : start + args.batch_size]
        subprocess.run(
            [
                "oxipng",
                "-o",
                str(args.level),
                "--strip",
                "safe",
                "--quiet",
                *map(str, batch),
            ],
            check=True,
        )
        done = min(start + len(batch), len(canonical))
        if done % 4096 < args.batch_size or done == len(canonical):
            print(f"  optimized {done}/{len(canonical)} unique PNGs")

    # oxipng atomically replaces optimized files. Reconnect every duplicate
    # animation path to its optimized canonical payload afterward.
    for paths in groups.values():
        canonical_path = paths[0]
        for alias in paths[1:]:
            alias.unlink()
            os.link(canonical_path, alias)

    after = physical_size(pngs)
    print(
        f"Physical PNG size: {before / 1024 / 1024:.1f} -> "
        f"{after / 1024 / 1024:.1f} MiB "
        f"({before - after:,} bytes saved)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
