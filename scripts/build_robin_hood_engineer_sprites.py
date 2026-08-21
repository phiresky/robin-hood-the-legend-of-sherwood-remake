#!/usr/bin/env python3
"""Build Factorio atlases from the port's hackable Robin Hood sprite data."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from PIL import Image, ImageDraw


CELL_SIZE = 160
PIVOT = (80, 120)
DIRECTIONS = tuple(range(0, 16, 2))
# Factorio's running-with-gun animation has 18 half-circle combinations.
# The engine reverses/mirrors combinations for the other half. Map those
# combinations to the nearest of Robin's original 16 directional renders.
GUN_RUNNING_DIRECTIONS = tuple(round(index * 8 / 17) for index in range(18))
GREEN_KEY = (0, 251, 0)
SHADOW_KEY = (0, 0, 255)

ANIMATIONS = {
    "idle": (("WaitingUpright",), DIRECTIONS),
    "running": (("RunningUpright",), DIRECTIONS),
    "running-gun": (("RunningUpright",), GUN_RUNNING_DIRECTIONS),
    "aiming-bow": (("AimingWithBow",), DIRECTIONS),
    "shooting-bow": (
        ("TransitionLoadingBow", "TransitionRaisingBow", "ShootingWithBow"),
        DIRECTIONS,
    ),
    "mining-sword": (("StrikingRightSword",), DIRECTIONS),
    "dead": (("BeingDead",), (8,)),
}


def arguments() -> argparse.Namespace:
    repo_root = Path(__file__).resolve().parents[2]
    default_source = (
        repo_root
        / "datadirs/fullgame_gog_hackable/Data/Characters/RobinHood.rhs.d"
    )
    parser = argparse.ArgumentParser(
        description="Regenerate the mod graphics from hackable datadir PNGs."
    )
    parser.add_argument(
        "--source",
        type=Path,
        default=default_source,
        help="RobinHood.rhs.d directory (default: %(default)s)",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=Path(__file__).resolve().parents[1]
        / "robin-hood-engineer"
        / "graphics",
        help="output graphics directory (default: %(default)s)",
    )
    return parser.parse_args()


def split_source(source: Image.Image) -> tuple[Image.Image, Image.Image]:
    rgba = source.convert("RGBA")
    body = Image.new("RGBA", rgba.size)
    shadow = Image.new("RGBA", rgba.size)
    body_pixels = body.load()
    shadow_pixels = shadow.load()

    for y in range(rgba.height):
        for x in range(rgba.width):
            red, green, blue, alpha = rgba.getpixel((x, y))
            rgb = (red, green, blue)
            if alpha == 0 or rgb == GREEN_KEY:
                continue
            if rgb == SHADOW_KEY:
                shadow_pixels[x, y] = (0, 0, 0, 92)
            else:
                body_pixels[x, y] = (red, green, blue, alpha)

    return body, shadow


def find_rows(manifest: dict, action: str, directions: tuple[int, ...]) -> list[dict]:
    profiles = manifest.get("profiles")
    if not profiles:
        raise RuntimeError("manifest contains no sprite profiles")
    rows = profiles[0].get("rows")
    if rows is None:
        raise RuntimeError("manifest profile contains no rows")

    by_direction = {
        row["direction"]: row
        for row in rows
        if row.get("action") == action and row.get("direction") in directions
    }
    missing = set(directions) - set(by_direction)
    if missing:
        raise RuntimeError(f"{action} is missing directions {sorted(missing)}")
    return [by_direction[direction] for direction in directions]


def paste_frame(
    atlas: Image.Image,
    frame: Image.Image,
    column: int,
    row: int,
    source_center: tuple[float, float],
    metadata: dict,
) -> None:
    pivot_x = source_center[0] - metadata["offset_x"]
    pivot_y = source_center[1] - metadata["offset_y"]
    x = round(column * CELL_SIZE + PIVOT[0] - pivot_x)
    y = round(row * CELL_SIZE + PIVOT[1] - pivot_y)

    cell_left = column * CELL_SIZE
    cell_top = row * CELL_SIZE
    if (
        x < cell_left
        or y < cell_top
        or x + frame.width > cell_left + CELL_SIZE
        or y + frame.height > cell_top + CELL_SIZE
    ):
        raise RuntimeError(
            f"frame does not fit its {CELL_SIZE}px cell: "
            f"position=({x - cell_left},{y - cell_top}), size={frame.size}"
        )
    atlas.alpha_composite(frame, (x, y))


def build_animation(
    source_dir: Path,
    output_dir: Path,
    manifest: dict,
    output_name: str,
    actions: tuple[str, ...],
    directions: tuple[int, ...],
) -> None:
    rows_by_action = [find_rows(manifest, action, directions) for action in actions]
    action_frame_counts = []
    for action, rows in zip(actions, rows_by_action, strict=True):
        frame_counts = {len(row["frames"]) for row in rows}
        if len(frame_counts) != 1:
            raise RuntimeError(
                f"{action} has inconsistent frame counts: {frame_counts}"
            )
        action_frame_counts.append(frame_counts.pop())
    frame_count = sum(action_frame_counts)

    profile = manifest["profiles"][0]
    source_center = (profile["center_x"], profile["center_y"])
    size = (CELL_SIZE * frame_count, CELL_SIZE * len(directions))
    body_atlas = Image.new("RGBA", size)
    shadow_atlas = Image.new("RGBA", size)

    for row_index in range(len(directions)):
        output_frame_index = 0
        for rows in rows_by_action:
            row = rows[row_index]
            row_dir = source_dir / row["path"]
            for metadata in row["frames"]:
                frame_path = row_dir / metadata["file"]
                if not frame_path.is_file():
                    raise FileNotFoundError(f"missing source frame: {frame_path}")
                with Image.open(frame_path) as source:
                    body, shadow = split_source(source)
                paste_frame(
                    body_atlas,
                    body,
                    output_frame_index,
                    row_index,
                    source_center,
                    metadata,
                )
                paste_frame(
                    shadow_atlas,
                    shadow,
                    output_frame_index,
                    row_index,
                    source_center,
                    metadata,
                )
                output_frame_index += 1

    body_atlas.save(output_dir / f"{output_name}.png", optimize=True)
    shadow_atlas.save(output_dir / f"{output_name}-shadow.png", optimize=True)
    print(
        f"{output_name}: {' + '.join(actions)}, {len(directions)} directions x "
        f"{frame_count} frames"
    )


def build_icon(source_dir: Path, output_dir: Path, manifest: dict) -> None:
    row = find_rows(manifest, "WaitingUpright", (8,))[0]
    metadata = row["frames"][0]
    with Image.open(source_dir / row["path"] / metadata["file"]) as source:
        body, _ = split_source(source)

    icon = Image.new("RGBA", (64, 64))
    draw = ImageDraw.Draw(icon)
    draw.ellipse((2, 2, 61, 61), fill=(25, 75, 35, 255), outline=(212, 172, 62, 255), width=3)
    body.thumbnail((48, 58), Image.Resampling.LANCZOS)
    icon.alpha_composite(body, ((64 - body.width) // 2, 5))
    icon.save(output_dir / "robin-hood.png", optimize=True)


def main() -> None:
    args = arguments()
    source_dir = args.source.resolve()
    output_dir = args.output.resolve()
    manifest_path = source_dir / "manifest.json"
    if not manifest_path.is_file():
        raise FileNotFoundError(f"missing hackable sprite manifest: {manifest_path}")

    with manifest_path.open(encoding="utf-8") as manifest_file:
        manifest = json.load(manifest_file)

    output_dir.mkdir(parents=True, exist_ok=True)
    for output_name, (actions, directions) in ANIMATIONS.items():
        build_animation(
            source_dir,
            output_dir,
            manifest,
            output_name,
            actions,
            directions,
        )
    build_icon(source_dir, output_dir, manifest)


if __name__ == "__main__":
    main()
