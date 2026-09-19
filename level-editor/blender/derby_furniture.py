"""Trim audited interior furniture render columns at their supporting floors.

The source objects remain hidden as reversible backups. Collision descriptions
are not edited. Run refine() before the covered/revealed projection pass.
"""
import math

import bmesh
import bpy
from mathutils import Vector
from mathutils.geometry import intersect_point_tri_2d


ROOMS = ((224, 180, tuple(range(232, 239))),
         (186, 110, tuple(range(242, 249))))
TAG = "derby_furniture_floor_clip"


def _source(collection, number):
    identifier = f"building-{number:03}"
    matches = [o for o in collection.all_objects if o.type == "MESH"
               and o.get("source_node") == identifier and not o.get(TAG)]
    if len(matches) != 1:
        raise ValueError(f"Expected one original for {identifier}, found {len(matches)}")
    return matches[0]


def _floor(obj, expected):
    obj.data.calc_loop_triangles()
    vertices = [obj.matrix_world @ v.co for v in obj.data.vertices]
    triangles = [[vertices[i] for i in t.vertices] for t in obj.data.loop_triangles]
    horizontal = [t for t in triangles if max(v.z for v in t) - min(v.z for v in t) < .001]
    if not horizontal:
        raise ValueError(f"No horizontal floor in {obj.name}")
    height = max(sum(v.z for v in t) / 3 for t in horizontal)
    if abs(height * math.cos(math.radians(35)) - expected) > .1:
        raise ValueError(f"Unexpected supporting floor height in {obj.name}: {height}")
    return height, [t for t in horizontal if abs(t[0].z - height) < .001]


def _clip(source, height):
    mesh = source.data.copy()
    mesh.name = source.data.name + " / above room floor"
    bm = bmesh.new()
    try:
        bm.from_mesh(mesh)
        bm.transform(source.matrix_world)
        # The imported atlas uses disconnected faces with subpixel corner
        # offsets. Join these seams before creating one closed floor cap.
        bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=.6)
        bmesh.ops.bisect_plane(bm, geom=list(bm.verts) + list(bm.edges) + list(bm.faces),
                              dist=.0001, plane_co=(0, 0, height),
                              plane_no=(0, 0, 1), clear_inner=True)
        boundary = [e for e in bm.edges if e.is_boundary]
        if any(abs(v.co.z - height) > .001 for e in boundary for v in e.verts):
            raise ValueError(f"Unclosed upper furniture shell: {source.name}")
        caps = bmesh.ops.holes_fill(bm, edges=boundary, sides=0)["faces"]
        for face in caps:
            face.material_index = 0
        bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
        if any(not e.is_manifold for e in bm.edges) or any(f.calc_area() < 1e-7 for f in bm.faces):
            raise ValueError(f"Invalid clipped furniture shell: {source.name}")
        bm.transform(source.matrix_world.inverted())
        bm.to_mesh(mesh)
        mesh.update()
    except BaseException:
        bpy.data.meshes.remove(mesh)
        raise
    finally:
        bm.free()
    return mesh


def refine():
    """Create or reuse the fourteen audited above-floor furniture shells."""
    collection = bpy.data.collections["Derby Working"]
    bpy.context.view_layer.update()
    report = []
    for floor_id, expected_height, furniture in ROOMS:
        floor = _source(collection, floor_id)
        height, triangles = _floor(floor, expected_height)
        for number in furniture:
            source = _source(collection, number)
            existing = [o for o in collection.all_objects if o.get(TAG)
                        and o.get("source_node") == source.get("source_node")]
            if existing:
                if len(existing) != 1 or abs(existing[0]["support_floor_scene_z"] - height) > .001:
                    raise ValueError(f"Conflicting prior furniture refinement: {source.name}")
                report.append({"source_node": source["source_node"], "status": "existing"})
                continue
            vertices = [source.matrix_world @ v.co for v in source.data.vertices]
            center = sum(vertices, Vector()) / len(vertices)
            if not any(intersect_point_tri_2d(center.xy, *(v.xy for v in t)) for t in triangles):
                raise ValueError(f"Furniture lies outside audited floor: {source.name}")
            if not min(v.z for v in vertices) < height < max(v.z for v in vertices):
                raise ValueError(f"Floor does not intersect furniture: {source.name}")
            mesh = _clip(source, height)
            replacement = source.copy()
            replacement.data = mesh
            replacement.name = source.name + " / above room floor"
            replacement[TAG] = True
            replacement["support_floor_source_node"] = floor["source_node"]
            replacement["support_floor_scene_z"] = height
            collection.objects.link(replacement)
            replacement.hide_render = False
            replacement.hide_viewport = False
            replacement.hide_set(False)
            source.hide_render = True
            source.hide_viewport = True
            report.append({"source_node": source["source_node"], "status": "created",
                           "floor_source_node": floor["source_node"],
                           "floor_scene_z": height, "faces": len(mesh.polygons)})
    bpy.context.view_layer.update()
    return report
