"""Remove redundant planar stump subdivisions without changing its silhouette."""
import bpy
import bmesh

ASSET = "derby-east-bailey-yard-prop"


def refine():
    candidates = [obj for obj in bpy.data.collections["Derby Working"].objects
                  if obj.type == "MESH" and obj.get("asset_group") == ASSET
                  and obj.get("yard_prop_component") == "stump"]
    if len(candidates) != 1:
        raise ValueError(f"Expected one chopping stump, found {len(candidates)}")
    obj = candidates[0]
    if obj.get("source_node") != "building-113":
        raise ValueError("Unexpected chopping block source identity")
    mesh = obj.data.copy()
    bm = bmesh.new()
    try:
        bm.from_mesh(mesh)
        before = {"vertices": len(bm.verts), "faces": len(bm.faces),
                  "volume": bm.calc_volume()}
        # Face-level projection no longer needs a dense grid on flat bark panels.
        # Coplanar slivers also destabilize the solid preview's shadow edges.
        bmesh.ops.dissolve_limit(bm, angle_limit=.001, verts=list(bm.verts),
                                edges=list(bm.edges), use_dissolve_boundaries=False)
        bm.normal_update()
        after = {"vertices": len(bm.verts), "faces": len(bm.faces),
                 "volume": bm.calc_volume(),
                 "nonmanifold_edges": sum(not e.is_manifold for e in bm.edges),
                 "degenerate_faces": sum(f.calc_area() < 1e-8 for f in bm.faces)}
        if after["nonmanifold_edges"] or after["degenerate_faces"]:
            raise ValueError(f"Invalid stump cleanup: {after}")
        if abs(after["volume"] - before["volume"]) > .001:
            raise ValueError("Planar cleanup unexpectedly changed stump volume")
        bm.to_mesh(mesh)
    finally:
        bm.free()
    obj.data = mesh
    # The tangent bark panels face the source at cosine 0.1516: a single
    # painted pixel expands more than sixfold there. Keep that evidence unknown.
    obj["projection_min_cosine"] = .2
    obj["round2_chopping_block"] = "planar-stump-cleanup-v1"
    return {"asset": ASSET, "before": before, "after": after,
            "source_node": obj["source_node"], "shape_changed": False,
            "projection_min_cosine": .2}
