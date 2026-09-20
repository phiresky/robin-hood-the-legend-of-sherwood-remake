"""Conservative outcrop volume with reviewed exterior silhouette controls.

The broad contour follows authored mask 120; it does not reproduce raster
stair steps. Heights remain an inferred sloping-rock profile, not recovered
depth. Run source projection and packet validation after applying this recipe.
"""

import math

import bmesh
import bpy
from mathutils import Vector


ASSET = "derby-south-approach-stone"
TAG = "round2-approach-stone-contour-v1"
# Screen x/y and inferred world z. Shared ridge controls are identical in both
# halves, maintaining the two stable source identities without an open seam.
TOP = (760, 2557, 29)
RIDGE = (760, 2573, 18)
FOOT = (760, 2589.8, 0)
OUTLINES = {
    "building-047": [
        TOP, (757, 2557, 28), (750, 2563, 21), (742, 2568, 15),
        (736, 2574, 10), (731, 2581, 4), (727.5, 2584, 1),
        (728, 2589, 0), (732, 2592.5, 0), (745, 2592.5, 0),
        (754, 2591, 0), FOOT, RIDGE,
    ],
    "building-048": [
        TOP, RIDGE, FOOT, (772, 2589, 0), (781, 2588, 0),
        (787, 2585, 1), (788.5, 2580, 3), (788.5, 2574, 7),
        (785, 2569, 12), (782, 2566, 16), (775, 2562, 22),
        (770, 2559, 26), (763, 2557, 29),
    ],
}
CENTERS = {"building-047": (748, 2577, 12),
           "building-048": (772, 2575, 13)}


def refine():
    """Replace only the two active owned meshes; preserve identity/transforms."""
    working = bpy.data.collections["Derby Working"]
    objects = [o for o in working.all_objects if o.type == "MESH"
               and not o.hide_render and o.get("asset_group") == ASSET]
    by_node = {o.get("source_node"): o for o in objects}
    if len(objects) != 2 or set(by_node) != set(OUTLINES):
        raise ValueError("Expected exactly the two active approach stone parts")
    if all(o.get("round2_approach_stone") == TAG for o in objects):
        return {"status": "existing", "tag": TAG}
    sine = math.sin(math.radians(35))
    cosine = math.cos(math.radians(35))

    def world(point):
        x, y, z = point
        return Vector((x, -(y + z * cosine) / sine, z))

    prepared = []
    for node, outline in OUTLINES.items():
        obj = by_node[node]
        boundary = [world(p) for p in outline]
        count = len(boundary)
        verts = boundary + [Vector((p.x, p.y, -.5)) for p in boundary]
        verts.append(world(CENTERS[node]))
        center = len(verts) - 1
        faces = [(i, (i + 1) % count, center) for i in range(count)]
        faces.append(tuple(reversed(range(count, count * 2))))
        faces.extend((i, count + i, count + (i + 1) % count,
                      (i + 1) % count) for i in range(count))
        inverse = obj.matrix_world.inverted()
        mesh = bpy.data.meshes.new(node + " reviewed rock contour")
        mesh.from_pydata([inverse @ p for p in verts], [], faces)
        mesh.update()
        bm = bmesh.new()
        bm.from_mesh(mesh)
        bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
        bmesh.ops.triangulate(bm, faces=list(bm.faces))
        defects = {"nonmanifold_edges": sum(not e.is_manifold for e in bm.edges),
                   "degenerate_faces": sum(f.calc_area() < 1e-7 for f in bm.faces)}
        volume = bm.calc_volume(signed=True)
        if any(defects.values()) or volume <= 0:
            bm.free()
            bpy.data.meshes.remove(mesh)
            raise ValueError(f"Invalid {node} rock: {defects}, volume={volume}")
        bm.to_mesh(mesh)
        bm.free()
        mesh.uv_layers.new(name="UVMap")
        prepared.append((obj, mesh, defects, volume))

    # Neutral material is temporary; the packet's modified pass reapplies
    # source ownership and leaves unseen surfaces neutral for review.
    material = bpy.data.materials.new("Approach stone source-projection pending")
    material.diffuse_color = (.3, .3, .3, 1)
    report = []
    for obj, mesh, defects, volume in prepared:
        mesh.materials.append(material)
        obj.data = mesh
        obj["round2_approach_stone"] = TAG
        obj["reviewed_occlusion_mask"] = 120
        report.append({"source_node": obj["source_node"],
                       "vertices": len(mesh.vertices), "faces": len(mesh.polygons),
                       "volume": volume, **defects})
    return {"status": "refined", "tag": TAG, "parts": report,
            "mask": 120, "mask_layer": 0,
            "limitations": ["Depth is conservatively inferred",
                            "Closed halves retain internal joining faces"]}
