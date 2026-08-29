#!/usr/bin/env python3
"""Reconstruct the cut blue, black, and green mounted-knight sprite sets.

The retail game ships aligned yellow/orange/red rider animation frames, plus
complete blue/orange/black/green colour families for six infantry types.  We
learn the game's palette substitutions from those infantry families and apply
them only to pixels which differ across the three aligned rider references.
"""

from __future__ import annotations

import argparse
import colorsys
from collections import Counter, defaultdict
from hashlib import sha256
import json
import os
from pathlib import Path
import shutil

from PIL import Image


TARGETS = {
    "blue": ("00", 0.61),
    "black": ("04", None),
    "green": ("05", 0.31),
}
TRAINING_FAMILIES = ("Guard A", "Soldier A", "Archer", "Crossbowman", "Soldier B", "Guard B")
KEY_COLOURS = {(0, 251, 0), (0, 0, 255)}


def png_paths(root: Path) -> list[Path]:
    return sorted(p for p in root.rglob("*.png") if p.is_file())


def rgb565(rgb: tuple[int, int, int]) -> tuple[int, int, int]:
    r, g, b = rgb
    r5, g6, b5 = r >> 3, g >> 2, b >> 3
    return (
        (r5 << 3) | (r5 >> 2),
        (g6 << 2) | (g6 >> 4),
        (b5 << 3) | (b5 >> 2),
    )


def fallback_recolour(rgb: tuple[int, int, int], target: str) -> tuple[int, int, int]:
    r, g, b = (channel / 255.0 for channel in rgb)
    _h, saturation, value = colorsys.rgb_to_hsv(r, g, b)
    if target == "black":
        # Keep enough highlight range for plate/barding folds to remain legible.
        out_value = 0.055 + value * 0.34
        out_saturation = min(saturation, 0.16)
        out_hue = 0.60
    else:
        out_hue = TARGETS[target][1]
        out_saturation = max(0.52, min(0.92, saturation))
        out_value = value * (0.92 if target == "blue" else 0.86)
    out = colorsys.hsv_to_rgb(out_hue, out_saturation, min(1.0, out_value))
    return rgb565(tuple(round(channel * 255) for channel in out))


def learn_palette(characters: Path, samples_per_family: int) -> dict[str, dict[tuple[int, int, int], tuple[int, int, int]]]:
    votes: dict[str, dict[tuple[int, int, int], Counter[tuple[int, int, int]]]] = {
        target: defaultdict(Counter) for target in TARGETS
    }
    sampled = 0
    for family in TRAINING_FAMILIES:
        orange = characters / f"{family}02.rhs.d"
        if not orange.is_dir():
            continue
        paths = png_paths(orange)
        stride = max(1, len(paths) // samples_per_family)
        for src_path in paths[::stride][:samples_per_family]:
            relative = src_path.relative_to(orange)
            target_paths = {
                target: characters / f"{family}{suffix}.rhs.d" / relative
                for target, (suffix, _hue) in TARGETS.items()
            }
            if not all(path.is_file() for path in target_paths.values()):
                continue
            with Image.open(src_path) as src_image:
                src = src_image.convert("RGB")
            targets = {}
            valid = True
            for target, path in target_paths.items():
                with Image.open(path) as image:
                    targets[target] = image.convert("RGB")
                valid &= targets[target].size == src.size
            if not valid:
                continue
            src_pixels = list(src.get_flattened_data())
            target_pixels = {target: list(image.get_flattened_data()) for target, image in targets.items()}
            for index, colour in enumerate(src_pixels):
                if colour in KEY_COLOURS:
                    continue
                for target in TARGETS:
                    votes[target][colour][target_pixels[target][index]] += 1
            sampled += 1
    if sampled == 0:
        raise RuntimeError("no aligned infantry colour-family frames found")
    print(f"Learned palette substitutions from {sampled} aligned infantry frames")
    return {
        target: {source: choices.most_common(1)[0][0] for source, choices in table.items()}
        for target, table in votes.items()
    }


def colour_distance(a: tuple[int, int, int], b: tuple[int, int, int]) -> int:
    return abs(a[0] - b[0]) + abs(a[1] - b[1]) + abs(a[2] - b[2])


def recolour_frame(
    yellow_path: Path,
    orange_path: Path,
    red_path: Path,
    palettes: dict[str, dict[tuple[int, int, int], tuple[int, int, int]]],
) -> dict[str, Image.Image]:
    with Image.open(yellow_path) as image:
        yellow = image.convert("RGBA")
    with Image.open(orange_path) as image:
        orange = image.convert("RGBA")
    with Image.open(red_path) as image:
        red = image.convert("RGBA")
    if yellow.size != orange.size or red.size != orange.size:
        raise RuntimeError(f"unaligned rider frame sizes at {orange_path}")

    yp = list(yellow.get_flattened_data())
    op = list(orange.get_flattened_data())
    rp = list(red.get_flattened_data())
    outputs = {target: [] for target in TARGETS}
    for ypx, opx, rpx in zip(yp, op, rp):
        source = opx[:3]
        is_key = source in KEY_COLOURS
        variable = max(
            colour_distance(ypx[:3], opx[:3]),
            colour_distance(opx[:3], rpx[:3]),
            colour_distance(ypx[:3], rpx[:3]),
        ) >= 12
        for target in TARGETS:
            if is_key or not variable:
                out = source
            else:
                mapped = palettes[target].get(source)
                if mapped is None or colour_distance(mapped, source) < 6:
                    mapped = fallback_recolour(source, target)
                out = rgb565(mapped)
            outputs[target].append((*out, opx[3]))

    result = {}
    for target, pixels in outputs.items():
        image = Image.new("RGBA", orange.size)
        image.putdata(pixels)
        result[target] = image
    return result


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--characters",
        type=Path,
        default=Path("datadirs/fullgame_gog_hackable/Data/Characters"),
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("output/generated-knights/Characters"),
    )
    parser.add_argument("--samples-per-family", type=int, default=500)
    parser.add_argument("--limit", type=int, help="process only this many paths (preview/testing)")
    parser.add_argument("--force", action="store_true")
    args = parser.parse_args()

    sources = {i: args.characters / f"Knight{i:02}.rhs.d" for i in (1, 2, 3)}
    if not all(path.is_dir() for path in sources.values()):
        raise SystemExit("Knight01/02/03 extracted directories are required")

    destinations = {
        "blue": args.output / "Knight00.rhs.d",
        "black": args.output / "Knight04.rhs.d",
        "green": args.output / "Knight05.rhs.d",
    }
    for destination in destinations.values():
        if destination.exists() and not args.force:
            raise SystemExit(f"output exists: {destination} (pass --force to replace it)")
        if destination.exists():
            shutil.rmtree(destination)
        destination.mkdir(parents=True)

    palettes = learn_palette(args.characters, args.samples_per_family)
    orange_paths = png_paths(sources[2])
    if args.limit is not None:
        orange_paths = orange_paths[: args.limit]

    # The RHS repeats bank frames across actions. Hard-link repeated generated
    # PNG payloads so the complete directory stays compact without changing
    # the manifest structure expected by the hackable datadir loader.
    generated_by_digest: dict[str, dict[str, Path]] = {target: {} for target in TARGETS}
    for index, orange_path in enumerate(orange_paths, start=1):
        relative = orange_path.relative_to(sources[2])
        yellow_path = sources[1] / relative
        red_path = sources[3] / relative
        if not yellow_path.is_file() or not red_path.is_file():
            raise RuntimeError(f"missing aligned rider reference for {relative}")
        digest = sha256(yellow_path.read_bytes() + orange_path.read_bytes() + red_path.read_bytes()).hexdigest()
        images = None
        for target, destination in destinations.items():
            out_path = destination / relative
            out_path.parent.mkdir(parents=True, exist_ok=True)
            cached = generated_by_digest[target].get(digest)
            if cached is not None:
                os.link(cached, out_path)
                continue
            if images is None:
                images = recolour_frame(yellow_path, orange_path, red_path, palettes)
            images[target].save(out_path, optimize=True)
            generated_by_digest[target][digest] = out_path
        if index % 500 == 0 or index == len(orange_paths):
            print(f"Processed {index}/{len(orange_paths)} animation-frame paths")

    if args.limit is None:
        for target, destination in destinations.items():
            manifest = json.loads((sources[2] / "manifest.json").read_text())
            manifest["pixel_format"] = "legacy_color_keys"
            manifest["generated_variant"] = target
            manifest["source"] = "Knight02 with aligned Knight01/Knight03 livery mask"
            (destination / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")

    print("Generated:")
    for target, destination in destinations.items():
        print(f"  {target}: {destination}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
