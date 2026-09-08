#!/usr/bin/env python3
"""Validate additive soldier profiles, hackable RHS references, and PNG data."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from PIL import Image


# The shipped CPF contains repeated filenames because original levels refer to
# these records by numeric index. Hackable JSON must use the explicit form for
# them; keeping this list here also prevents validation from blessing a name
# that the runtime would reject as ambiguous.
RETAIL_DUPLICATE_PROFILE_INDICES = {
    "archer05": {17, 47},
    "crossbowman05": {41, 51},
    "guard_a05": {5, 45},
    "guard_b05": {35, 50},
    "knight02": {52, 54, 56, 57, 58},
    "officer02": {63, 64, 65},
    "officer05": {23, 48},
    "soldier_a05": {11, 46},
    "soldier_b05": {29, 49},
}


def identifier(filename: str) -> str:
    result = []
    separator = False
    for character in filename.lower():
        if character.isascii() and character.isalnum():
            if separator and result:
                result.append("_")
            result.append(character)
            separator = False
        else:
            separator = True
    return "".join(result)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("mods", nargs="+", type=Path)
    parser.add_argument(
        "--retail-characters",
        type=Path,
        default=Path("datadirs/fullgame_gog/DATA/Characters"),
        help="retail character directory used to validate non-additive profile references",
    )
    args = parser.parse_args()

    retail_identifiers = {
        identifier(path.stem) for path in args.retail_characters.glob("*.rhs")
    }
    if not retail_identifiers:
        raise RuntimeError(f"no retail RHS profiles found in {args.retail_characters}")
    retail_identifiers -= RETAIL_DUPLICATE_PROFILE_INDICES.keys()
    retail_identifiers |= {
        f"{base}__{index}"
        for base, indices in RETAIL_DUPLICATE_PROFILE_INDICES.items()
        for index in indices
    }

    profile_count = 0
    frame_references = 0
    pngs: set[Path] = set()
    for root in args.mods:
        patch_path = root / "Data/Configuration/soldier-profiles.patch.json"
        additions = json.loads(patch_path.read_text())["soldiers"]
        filenames = {addition["filename"] for addition in additions}
        identifiers = {identifier(filename) for filename in filenames}
        if len(identifiers) != len(filenames):
            raise RuntimeError(f"duplicate normalized profile identifier in {patch_path}")
        for addition in additions:
            for field in ("template", "progression_from"):
                reference = addition.get(field)
                if reference is not None and reference not in retail_identifiers:
                    raise RuntimeError(
                        f"{patch_path}: {field} uses unknown or ambiguous retail "
                        f"profile {reference!r}"
                    )

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
    print(
        f"validated {profile_count} profiles, {frame_references} frame references, "
        f"and {len(pngs)} PNG files"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
