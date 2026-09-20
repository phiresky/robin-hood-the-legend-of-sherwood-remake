"""Complete the watchtower's rear crenels and eave-height turret roof.

The existing three front embrasures and three roof-access steps are retained.
Run before layered reprojection; collision columns remain separate and unchanged.
"""
import math
import bpy
import bmesh
from mathutils import Vector
from derby_asset_lower_east_curtain import _rebuild
from derby_asset_east_hall import _refine
from derby_asset_east_bailey_gate import _stone_fallback, _roof_fallback, _replacement
from derby_asset_great_keep import _roof_panel

ASSET = "derby-east-watchtower"
TAG = "east_watchtower_asset_refinement"


def _audit(obj):
    bm = bmesh.new(); bm.from_mesh(obj.data)
    try:
        return {"nonmanifold_edges": sum(not e.is_manifold for e in bm.edges),
                "degenerate_faces": sum(f.calc_area() < 1e-7 for f in bm.faces)}
    finally:
        bm.free()


def _trim_turret(obj, height):
    bm = bmesh.new(); bm.from_mesh(obj.data)
    try:
        bmesh.ops.bisect_plane(bm, geom=list(bm.verts) + list(bm.edges) + list(bm.faces),
                              plane_co=(0, 0, height), plane_no=(0, 0, 1),
                              dist=.0001, clear_inner=True)
        bmesh.ops.holes_fill(bm, edges=[e for e in bm.edges if e.is_boundary], sides=0)
        bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
        bm.to_mesh(obj.data)
    finally:
        bm.free()


def refine():
    working = bpy.data.collections["Derby Working"]
    bpy.context.view_layer.update()
    active = [o for o in working.all_objects if o.type == "MESH" and not o.hide_render
              and o.get("asset_group") == ASSET]
    if active and all(o.get(TAG) for o in active):
        return {"status": "existing", "objects": len(active)}
    if any(o.get(TAG) for o in active):
        raise ValueError("Partially refined watchtower; inspect checkpoint before continuing")
    sources = {}
    for number in range(213, 223):
        objects = [o for o in active if o.get("source_node") == f"building-{number:03}"
                   and not o.get("step_count")]
        if len(objects) != 1:
            raise ValueError(f"Expected one watchtower source {number}, found {len(objects)}")
        sources[number] = objects[0]
    if sources[215].get("embrasure_count") != 3:
        raise ValueError("The three reviewed front embrasures must be applied first")
    world = {n: [o.matrix_world @ v.co for v in o.data.vertices] for n, o in sources.items()}
    original_open = _audit(sources[215])["nonmanifold_edges"]
    _refine(sources[215], [
        (114, 127, 74, 71, [(1718, 1726)], 24),
        (119, 111, 71, 121, [(1762, 1773)], 24),
        (111, 110, 121, 69, [(1785, 1794)], 24),
        (110, 107, 69, 128, [(1805, 1815)], 24),
    ])
    battlements = bpy.data.objects[sources[215]["replaced_by"]]
    battlements["embrasure_count"] = 7
    battlements[TAG] = True
    battlements["projection_min_cosine"] = .35
    _stone_fallback(battlements)
    bm = bmesh.new(); bm.from_mesh(battlements.data)
    bmesh.ops.triangulate(bm, faces=list(bm.faces))
    bmesh.ops.subdivide_edges(bm, edges=list(bm.edges), cuts=2, use_grid_fill=True)
    bm.to_mesh(battlements.data); bm.free()
    # Main tower and lower interfaces: continuous source top outlines close
    # missing concealed wall panels while preserving their measured footprint.
    for number in (213, 214, 217, 218, 219):
        source = sources[number]
        _rebuild(source, [])
        obj = next(o for o in working.all_objects if o.get("source_node") == source["source_node"]
                   and o != source and not o.hide_render)
        if number == 219:
            _trim_turret(obj, max(p.z for p in world[214]))
        obj.name = source.name + " / closed shell"
        _stone_fallback(obj)
        if number == 214:
            bm = bmesh.new(); bm.from_mesh(obj.data); bm.normal_update()
            edges = {edge for face in bm.faces if face.normal.z > .9 for edge in face.edges}
            bmesh.ops.subdivide_edges(bm, edges=list(edges), cuts=3, use_grid_fill=True)
            bm.to_mesh(obj.data); bm.free()
        obj[TAG] = True
        source.hide_viewport = True
    # The cap silhouette is curved in the artwork. Three canonical roof parts
    # share a complete 36-sector roof instead of opaque ground-reaching wedges.
    center = Vector((1846.30, -2435.0, 0))
    rx, ry = 40.5, 41.5
    profiles = ((661.6, 1.07), (690, .78), (744, .25), (777.39, 0))
    triangles = {220: [], 221: [], 222: []}
    for sector in range(36):
        owner = (220, 221, 222)[sector // 12]
        a, b = 2 * math.pi * sector / 36, 2 * math.pi * (sector + 1) / 36
        for (z0, r0), (z1, r1) in zip(profiles, profiles[1:]):
            def point(angle, z, r):
                return Vector((center.x + rx * r * math.cos(angle), center.y + ry * r * math.sin(angle), z))
            p, q = point(a, z0, r0), point(b, z0, r0)
            s, t = point(a, z1, r1), point(b, z1, r1)
            triangles[owner].append((p, q, s))
            if r1:
                triangles[owner].append((q, t, s))
    for number in (220, 221, 222):
        source = sources[number]
        _roof_panel(source, triangles[number], [])
        obj = next(o for o in working.all_objects if o.get("source_node") == source["source_node"]
                   and not o.hide_render)
        obj[TAG] = True
        _roof_fallback(obj)
        obj["projection_min_cosine"] = .35
        source.hide_render = source.hide_viewport = True
    for obj in active:
        if obj.get("source_node") == "building-216":
            if obj.get("step_count"):
                obj[TAG] = True
            else:
                mesh = obj.data.copy()
                bm = bmesh.new(); bm.from_mesh(mesh)
                bm.transform(obj.matrix_world)
                bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=.6)
                bmesh.ops.holes_fill(bm, edges=[e for e in bm.edges if e.is_boundary], sides=0)
                bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
                bm.to_mesh(mesh); bm.free()
                replacement = _replacement(obj, mesh)
                replacement[TAG] = True
    final = [o for o in working.all_objects if o.type == "MESH" and not o.hide_render
             and o.get("asset_group") == ASSET]
    report = []
    for obj in final:
        validation = _audit(obj)
        if validation["degenerate_faces"] or (obj != battlements and validation["nonmanifold_edges"]
                                             and obj.get("source_node") != "building-216"):
            raise ValueError(f"Invalid watchtower shell {obj.name}: {validation}")
        report.append({"source_node": obj["source_node"], "name": obj.name, **validation})
    bpy.context.view_layer.update()
    return {"status": "refined", "asset": ASSET, "embrasures": 7,
            "inherited_battlement_open_edges_before": original_open, "objects": report}
