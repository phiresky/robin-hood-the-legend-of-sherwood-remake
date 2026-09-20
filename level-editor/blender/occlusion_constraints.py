"""Reviewed source silhouettes; character masks are never auto-associated.

Manifest schema (paths relative to this manifest):
{
  "version": 1,
  "mask_inventory": "../../Data/Levels/Derby.rhp.d/masks/manifest.json",
  "projections": {
    "exterior": {
      "source_sha256": "64 lowercase hex characters for the exact source PNG",
      "state": "covered",
      "assignments": [
        {"reviewed": true, "asset_group": "map-cottage", "mask_indices": [10, 11]},
        {"reviewed": true, "source_node": "building-053", "mask_indices": [10]}
      ]
    }
  }
}

Each assignment unions its masks. A source_node assignment overrides its asset
group assignment. Labels absent from projections, and objects without an explicit
assignment, remain unconstrained. A present label requires an exact source hash,
nonempty state description and reviewed=true on EVERY assignment. State names
document the reviewed artwork; a label/hash pair is the machine checked guard.
Native converter inventories use index, box_top_left, box_size and png (relative
to the inventory directory). Degenerate entries with png=null cannot constrain a
surface. Review-export inventories using folder/mask.png are also supported.
White pixels are owned; black pixels are outside the reviewed silhouette.
These masks constrain texture evidence only; they never modify geometry.
"""
import json
from pathlib import Path


def _load_bitmap(path):
    import bpy
    import numpy as np
    image = bpy.data.images.load(str(path), check_existing=False)
    try:
        width, height = image.size
        pixels = np.empty(width * height * 4, dtype=np.float32)
        image.pixels.foreach_get(pixels)
        # Inventory boxes have a top-left origin; Blender image arrays do not.
        rgba = pixels.reshape(height, width, 4)[::-1]
        return (rgba[:, :, :3].max(axis=2) > .5) & (rgba[:, :, 3] > .5)
    finally:
        bpy.data.images.remove(image)


class SourceMaskConstraints:
    def __init__(self, manifest_path, projection_label, source_sha256, source_size,
                 image_loader=None):
        self.path = Path(manifest_path).resolve()
        self.assignment_by_node = {}
        self.assignment_by_group = {}
        self.source_size = source_size
        manifest = json.loads(self.path.read_text())
        if manifest.get("version") != 1:
            raise ValueError("Unsupported source-mask manifest version")
        projection = manifest["projections"].get(projection_label)
        self.active = projection is not None
        self.state = None
        if projection is None:
            return
        if projection.get("source_sha256") != source_sha256:
            raise ValueError("Reviewed source-mask hash does not match projection artwork")
        self.state = projection.get("state")
        if not isinstance(self.state, str) or not self.state.strip():
            raise ValueError("Source-mask projection requires a reviewed state description")
        inventory_path = (self.path.parent / manifest["mask_inventory"]).resolve()
        inventory = json.loads(inventory_path.read_text())
        records = {}
        for record in inventory["masks"]:
            index = record["index"]
            if index in records:
                raise ValueError(f"Duplicate occlusion-mask index {index}")
            records[index] = record
        cache = {}
        loader = image_loader or _load_bitmap
        for assignment in projection["assignments"]:
            if assignment.get("reviewed") is not True:
                raise ValueError("Unreviewed source-mask assignment cannot constrain projection")
            kinds = [key for key in ("source_node", "asset_group") if key in assignment]
            if len(kinds) != 1 or not isinstance(assignment[kinds[0]], str) or not assignment[kinds[0]]:
                raise ValueError("Source-mask assignment requires exactly one explicit target")
            indices = assignment.get("mask_indices")
            if not isinstance(indices, list) or not indices:
                raise ValueError("Source-mask assignment requires mask indices")
            masks = []
            for index in indices:
                if type(index) is not int or index not in records:
                    raise ValueError(f"Unknown occlusion-mask index {index!r}")
                if index not in cache:
                    record = records[index]
                    left, top = record["box_top_left"]
                    width, height = record["box_size"]
                    if any(type(v) is not int for v in (left, top, width, height)) or min(width, height) <= 0:
                        raise ValueError(f"Invalid mask bounding box {index}")
                    if "png" in record:
                        if not isinstance(record["png"], str) or not record["png"]:
                            raise ValueError(f"Mask {index} has no bitmap")
                        bitmap_path = inventory_path.parent / record["png"]
                    else:
                        bitmap_path = inventory_path.parent / record["folder"] / "mask.png"
                    bitmap = loader(bitmap_path)
                    if bitmap.shape != (height, width):
                        raise ValueError(f"Mask {index} dimensions differ from its inventory box")
                    cache[index] = (left, top, bitmap)
                masks.append(cache[index])
            target = assignment[kinds[0]]
            mapping = self.assignment_by_node if kinds[0] == "source_node" else self.assignment_by_group
            if target in mapping:
                raise ValueError(f"Duplicate source-mask assignment {target}")
            mapping[target] = masks

    def for_object(self, obj):
        return self.assignment_by_node.get(obj.get("source_node"),
                                           self.assignment_by_group.get(obj.get("asset_group")))

    def allowed(self, masks, sx, sy):
        """Return union membership for bottom-origin source pixel coordinates."""
        import numpy as np
        if masks is None:
            return np.ones(len(sx), dtype=bool)
        accepted = np.zeros(len(sx), dtype=bool)
        top_y = self.source_size[1] - 1 - sy
        for left, top, bitmap in masks:
            x, y = sx-left, top_y-top
            inside = (x >= 0) & (x < bitmap.shape[1]) & (y >= 0) & (y < bitmap.shape[0])
            accepted[inside] |= bitmap[y[inside], x[inside]]
        return accepted
