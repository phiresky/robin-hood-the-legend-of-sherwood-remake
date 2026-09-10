#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["Pillow>=10", "jsonpatch>=1.33,<2"]
# ///
"""Validate JSON-patched profiles, hackable RHS references, and PNG data.

Run with uv; dependencies are declared inline above.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from PIL import Image
import jsonpatch

from profile_patch_tools import identifier, load_catalog


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("mods", nargs="+", type=Path)
    parser.add_argument("--profile-catalog", type=Path, default=Path("target/profiles.patch-view.json"))
    parser.add_argument("--profiles-only", action="store_true",
                        help="validate patched profiles and mission references without rechecking sprite images")
    args = parser.parse_args()

    catalog = load_catalog(args.profile_catalog)
    retail_identifiers = {
        identifier(profile["filename"]) + (
            "__" + key.rsplit("#", 1)[1] if key != profile["filename"] else ""
        )
        for key, profile in catalog["soldiers"].items()
    }

    profile_count = 0
    frame_references = 0
    pngs: set[Path] = set()
    for root in args.mods:
        patch_path = root / "Data/Configuration/profiles.patch.json"
        operations = json.loads(patch_path.read_text())
        if not isinstance(operations, list):
            raise RuntimeError(f"{patch_path}: expected an RFC 6902 operation array")
        patched = jsonpatch.apply_patch(catalog, operations)
        additions = [profile for key, profile in patched["soldiers"].items() if key not in catalog["soldiers"]]
        filenames = {addition["filename"] for addition in additions}
        identifiers = {identifier(filename) for filename in filenames}
        if len(filenames) != len(additions) or len(identifiers) != len(filenames):
            raise RuntimeError(f"duplicate added profile identifier in {patch_path}")

        level_files = list((root / "Data/Levels").glob("*.level.json"))
        if not level_files:
            raise RuntimeError(f"no hackable level descriptor in {root}")
        for level_path in level_files:
            level = json.loads(level_path.read_text())
            for soldier in level["soldiers"]:
                if soldier["profile"] not in identifiers | retail_identifiers:
                    raise RuntimeError(
                        f"{level_path}: unknown added or retail profile {soldier['profile']!r}"
                    )

        if args.profiles_only:
            profile_count += len(filenames)
            continue

        for filename in filenames:
            rhs = root / "Data/Characters" / f"{filename}.rhs.d"
            manifest_path = rhs / "manifest.json"
            manifest = json.loads(manifest_path.read_text())
            if manifest["pixel_format"] != "legacy_color_keys":
                raise RuntimeError(f"{manifest_path}: expected legacy_color_keys")
            profile_count += 1
            for profile in manifest["profiles"]:
                for row in profile["rows"]:
                    for frame in row["frames"]:
                        path = rhs / row["path"] / frame["file"]
                        if not path.is_file():
                            raise RuntimeError(f"missing frame: {path}")
                        pngs.add(path)
                        frame_references += 1

    for index, path in enumerate(sorted(pngs), 1):
        with Image.open(path) as image:
            image.verify()
        if index % 25_000 == 0:
            print(f"verified {index}/{len(pngs)} PNGs")
    if args.profiles_only:
        print(f"validated {profile_count} patched profiles and their mission references")
        return 0
    print(
        f"validated {profile_count} profiles, {frame_references} frame references, "
        f"and {len(pngs)} PNG files"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
