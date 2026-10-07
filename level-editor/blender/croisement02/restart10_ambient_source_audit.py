"""Measure complete native ambient frame domains before physical binding."""
import hashlib
import json
from pathlib import Path

import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[3]
WORK = ROOT / "level-editor/work/croisement02-refinement"


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    source = WORK / "animation-references/manifest.json"
    inventory = WORK / "canopy-phase-receiver-inventory-v1/manifest.json"
    receivers = {g["index"]: g for g in json.loads(inventory.read_text())["groups"]}
    records = []
    for animation in json.loads(source.read_text())["animations"]:
        frames = animation["frames"]
        left = min(f["bbox"][0] for f in frames)
        top = min(f["bbox"][1] for f in frames)
        right = max(f["bbox"][0] + f["bbox"][2] for f in frames)
        bottom = max(f["bbox"][1] + f["bbox"][3] for f in frames)
        union = np.zeros((bottom - top, right - left), dtype=bool)
        first = None
        rows = []
        for index, frame in enumerate(frames):
            path = Path(frame["image"])
            source_path = Path(frame["source"])
            assert sha(source_path) == frame["sha256"], source_path
            rgba = np.asarray(Image.open(path).convert("RGBA"))
            original = np.asarray(Image.open(source_path).convert("RGBA"))
            # The inspection export converts the native green color key to alpha.
            key = np.all(original[:, :, :3] == (0, 251, 0), axis=2)
            assert np.array_equal(rgba[:, :, :3][~key], original[:, :, :3][~key]), path
            assert np.array_equal(rgba[:, :, 3], np.where(key, 0, original[:, :, 3])), path
            x, y, width, height = frame["bbox"]
            assert rgba.shape == (height, width, 4), path
            alpha = rgba[:, :, 3] > 0
            current = np.zeros_like(union)
            current[y - top:y - top + height, x - left:x - left + width] = alpha
            union |= current
            if first is None:
                first = current.copy()
            ys, xs = np.nonzero(alpha)
            rows.append({"index": index, "file": str(path), "sha256": sha(path),
                         "source": str(source_path), "source_sha256": frame["sha256"],
                         "visible_source_rgb_exact": True, "native_color_key_alpha_exact": True,
                         "duration_ticks": frame["delay"] + 1,
                         "opaque_pixels": int(alpha.sum()),
                         "alpha_centroid_display": [float(xs.mean() + x), float(ys.mean() + y)] if len(xs) else None,
                         "bbox": frame["bbox"]})
        records.append({"index": animation["index"], "profile": animation["profile"],
                        "kind": animation["kind"], "sprite": animation["sprite"],
                        "cycle_ticks": sum(r["duration_ticks"] for r in rows),
                        "temporal_bbox": [left, top, right - left, bottom - top],
                        "union_pixels": int(union.sum()),
                        "later_pixels_outside_first": int((union & ~first).sum()),
                        "source_associated_assets": [a["asset"] for a in receivers.get(animation["index"], {}).get("assets", [])],
                        "frames": rows})
    destination = WORK / "restart10-ambient-source-audit-v1"
    destination.mkdir(exist_ok=True)
    report = {"status": "Source-domain audit only; physical delivery remains incomplete",
              "source_sha256": sha(source), "receiver_inventory_sha256": sha(inventory),
              "animations": records,
              "constraints": [
                  "Source frame counters use serialized delay plus one ticks; alpha centroid is image evidence, not a measured 3D trajectory.",
                  "Sprite elevation participates in display ordering; it does not establish butterfly flight height or crown geometry.",
                  "Butterfly05 has elevation zero and requires distinct background restore/empty-frame handling.",
                  "Eight source canopy clocks cover44 associated crowns; current model hashes and temporal neighboring ownership still require binding.",
                  "Tree21 has no observed canopy sequence; fringe22 association is unresolved. Hidden animation must be labeled inferred.",
                  "Native camera overlays alone do not establish arbitrary-view physical playback."
              ]}
    (destination / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps([{k: r[k] for k in ("index", "kind", "cycle_ticks", "union_pixels", "later_pixels_outside_first")} for r in records]))


if __name__ == "__main__":
    main()
