"""Approximate one layer's initial character occlusion as a unified 16-bit PNG.

Usage: python3 level-editor/refinement/masks_to_depth.py --level <map>.rhp.json
       --map-image <map>.map.png --output work/<map>.occlusion-depth.png

This is an occlusion-threshold field, not recovered 3D geometry. A mask is
selected using the actor anchor X; a texture instead samples each sprite pixel
X. Sloping thresholds and finite polyline extents therefore cannot be reproduced
exactly. Projectile tests, other layers and subsequent patch states are excluded.
"""

import argparse
import json
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFont


def character_threshold(polyline, columns):
    """Match the piecewise linear threshold, including duplicate-X endpoints."""
    points = np.asarray(polyline, dtype=np.float64)
    if points.ndim != 2 or points.shape[0] < 2 or points.shape[1] != 2:
        raise ValueError("Character polyline requires at least two XY points")
    if not np.isfinite(points).all() or np.any(np.diff(points[:, 0]) < 0):
        raise ValueError("Character polyline must be finite and sorted by X")
    valid = (columns >= points[0, 0]) & (columns <= points[-1, 0])
    indices = np.clip(np.searchsorted(points[:, 0], columns, side="left"), 1, len(points) - 1)
    a, b = points[indices - 1], points[indices]
    dx = b[:, 0] - a[:, 0]
    fraction = np.divide(columns - a[:, 0], dx, out=np.zeros_like(columns, dtype=float), where=dx != 0)
    threshold = np.where(dx == 0, np.maximum(a[:, 1], b[:, 1]), a[:, 1] + fraction * (b[:, 1] - a[:, 1]))
    return np.where(valid, threshold, 0)


def initial_mask_indices(level):
    layers = {}
    for index, mask in enumerate(level["masks"]):
        layers.setdefault(mask["layer"], []).append(index)
    inactive = set()
    for patch in level["patches"]:
        for ref in patch["new_masks"]:
            if ref["index"] < 0:
                raise ValueError("Negative patch mask index")
            inactive.add(layers[ref["layer"]][ref["index"]])
    return set(range(len(level["masks"]))) - inactive


def convert(level_path, map_image, output, layer=0, mask_ids=False):
    level = json.loads(level_path.read_text())
    masks_dir = level_path.with_suffix("").with_suffix(".rhp.d") / "masks"
    manifest = json.loads((masks_dir / "manifest.json").read_text())
    entries = {entry["index"]: entry for entry in manifest["masks"]}
    with Image.open(map_image) as image:
        width, height = image.size
    result = np.zeros((height, width), dtype=np.uint16)
    labels = []
    included = 0
    active = initial_mask_indices(level)
    for index, mask in enumerate(level["masks"]):
        if index not in active or mask["layer"] != layer or not mask["mask_type"] & 1:
            continue
        x, y = mask["box_top_left"]
        w, h = mask["box_size"]
        if not w or not h:
            continue
        path = entries[index]["png"]
        if not path or Path(path).is_absolute() or ".." in Path(path).parts:
            raise ValueError(f"Unsafe or missing PNG path for mask {index}")
        with Image.open(masks_dir / path) as image:
            if image.size != (w, h):
                raise ValueError(f"Mask {index} PNG dimensions do not match level JSON")
            pixels = np.asarray(image.convert("L")) != 0
        left, top, right, bottom = max(0, x), max(0, y), min(width, x + w), min(height, y + h)
        if right <= left or bottom <= top:
            continue
        threshold = character_threshold(mask["character_polyline"], np.arange(left, right, dtype=float) + 0.5)
        encoded = np.rint(np.clip(threshold / height, 0, 1) * 65535).astype(np.uint16)
        coverage = pixels[top - y:bottom - y, left - x:right - x]
        target = result[top:bottom, left:right]
        np.maximum(target, np.where(coverage, encoded[None, :], 0), out=target)
        if mask_ids:
            rows, cols = np.nonzero(coverage)
            if len(rows):
                # Anchor on a covered pixel nearest the coverage centroid.
                nearest = np.argmin((rows - rows.mean()) ** 2 + (cols - cols.mean()) ** 2)
                labels.append((index, left + int(cols[nearest]), top + int(rows[nearest])))
        included += 1
    if not included:
        raise ValueError(f"No nonempty initial character masks found on layer {layer}")
    output.parent.mkdir(parents=True, exist_ok=True)
    Image.fromarray(result).save(output)
    report = {
        "source": str(level_path), "layer": layer, "state": "initial, before mission scripts",
        "size": [width, height], "included_masks": included,
        "covered_pixels": int(np.count_nonzero(result)),
        "encoding": "round(clamp(character_polyline_y / map_height, 0, 1) * 65535); zero = uncovered; overlaps = maximum",
        "limitations": [
            "Threshold field, not physical 3D depth.",
            "Polyline selection uses sprite-pixel X instead of actor-anchor X; sloped thresholds and endpoints are approximate.",
            "Only one layer and initial proto-level patch state; mission script changes are not evaluated.",
            "Does not reproduce projectile/obstacle tests. The depth renderer also applies a two-pixel comparison bias.",
        ],
    }
    if mask_ids:
        preview_path = output.with_name(output.stem + ".mask-ids.png")
        preview = Image.fromarray((result / 257).astype(np.uint8)).convert("RGB")
        draw = ImageDraw.Draw(preview)
        font = ImageFont.load_default(size=16)
        for index, x, y in labels:
            text = str(index)
            box = draw.textbbox((0, 0), text, font=font)
            label_width, label_height = box[2] - box[0], box[3] - box[1]
            tx = max(2, min(width - label_width - 2, x - label_width // 2))
            ty = max(2, min(height - label_height - 2, y - label_height // 2))
            draw.rectangle((tx - 2, ty - 2, tx + label_width + 2, ty + label_height + 2), fill="black")
            draw.text((tx - box[0], ty - box[1]), text, font=font, fill="yellow")
        preview.save(preview_path)
        report["mask_id_preview"] = str(preview_path)
        report["mask_id_labels"] = [
            {"index": index, "anchor": [x, y]} for index, x, y in labels
        ]
        report["mask_id_convention"] = "Zero-based global index in level masks array and mask manifest; not layer-local patch index. Labels may overlap in dense regions."
    output.with_suffix(".json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--level", required=True, type=Path)
    parser.add_argument("--map-image", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--layer", type=int, default=0)
    parser.add_argument("--mask-ids", action="store_true", help="Also write an RGB .mask-ids.png preview labeled with global mask indices")
    args = parser.parse_args()
    convert(args.level, args.map_image, args.output, args.layer, args.mask_ids)


if __name__ == "__main__":
    main()
