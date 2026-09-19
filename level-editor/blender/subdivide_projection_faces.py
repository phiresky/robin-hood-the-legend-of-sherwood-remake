"""Bounded surface subdivision for partly occluded projection receivers."""
import math

import bpy
import bmesh


def subdivide_tables(map_name="Derby", spacing=8.0):
    """Split the two audited banquet tables without moving their surfaces.

    Shared edges subdivide together to keep the closed furniture shells intact.
    Smaller faces let visibility reject only locally occluded source regions.
    """
    if spacing <= 0:
        raise ValueError("Projection cell spacing must be positive")
    if map_name != "Derby":
        raise ValueError("Table receiver IDs are audited for Derby only")
    bpy.context.view_layer.update()
    sources = [o for o in bpy.data.collections[map_name + " Working"].all_objects
               if o.type == "MESH" and not o.hide_render
               and o.get("source_node") in ("building-232", "building-244")]
    if len(sources) != 2:
        raise ValueError("Expected both audited banquet tables")
    if any(not obj.get("derby_furniture_floor_clip") for obj in sources):
        raise ValueError("Clip furniture to its audited room floor before subdivision")
    reports = []
    for obj in sources:
        if obj.get("projection_subdivision_spacing"):
            if obj["projection_subdivision_spacing"] != spacing:
                raise ValueError("Restore the unsplit table before changing spacing")
            reports.append({"object": obj.name, "already_subdivided": True})
            continue
        mesh = obj.data.copy()
        fallback = mesh.attributes.get("reprojection_fallback_material")
        if fallback:
            for face in mesh.polygons:
                face.material_index = fallback.data[face.index].value
            mesh.attributes.remove(fallback)
        bm = bmesh.new()
        try:
            bm.from_mesh(mesh)
            longest = max((obj.matrix_world.to_3x3() @ (e.verts[0].co - e.verts[1].co)).length
                          for e in bm.edges)
            cuts = min(20, max(1, math.ceil(longest / spacing) - 1))
            before = len(bm.faces)
            bmesh.ops.subdivide_edges(bm, edges=list(bm.edges), cuts=cuts,
                                     use_grid_fill=True, smooth=0.0)
            bm.normal_update()
            if any(not e.is_manifold for e in bm.edges):
                raise ValueError(f"Subdivision opened table shell: {obj.name}")
            if any(f.calc_area() < 1e-8 for f in bm.faces):
                raise ValueError(f"Subdivision created degenerate face: {obj.name}")
            bm.to_mesh(mesh)
        finally:
            bm.free()
        mesh.update()
        obj.data = mesh
        obj["projection_subdivision_spacing"] = spacing
        reports.append({"object": obj.name, "faces_before": before,
                        "faces_after": len(mesh.polygons), "edge_cuts": cuts})
    return reports
