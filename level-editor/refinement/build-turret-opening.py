"""Build an unpublished doorway mesh candidate without editing library assets."""
import hashlib
import json
import math
from pathlib import Path
import sys

import bpy
import bmesh
from mathutils import Vector

config_path = Path(sys.argv[sys.argv.index("--") + 1]).resolve()
config = json.loads(config_path.read_text())
source = Path(config["inputModel"])
assert hashlib.sha256(source.read_bytes()).hexdigest() == config["source"]["model_sha256"]
output = Path(config["outputModel"])
assert not output.exists(), "Do not overwrite an existing candidate"
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=str(source))


def prism(name, points):
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata(points, [], [(3, 2, 1, 0), (4, 5, 6, 7),
                                (0, 1, 5, 4), (1, 2, 6, 5),
                                (2, 3, 7, 6), (3, 0, 4, 7)])
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bm.to_mesh(mesh)
    bm.free()
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.collection.objects.link(obj)
    return obj


def join_stacked_shells(obj):
    """Union the two touching closed wall shells before cutting their doorway."""
    joint = 356 / math.cos(math.radians(config["camera"]["elevation_deg"]))
    upper = obj.copy()
    upper.data = obj.data.copy()
    bpy.context.collection.objects.link(upper)
    bpy.context.view_layer.update()
    for piece, lower in [(obj, True), (upper, False)]:
        bm = bmesh.new()
        bm.from_mesh(piece.data)
        bm.normal_update()
        normals = piece.matrix_world.to_3x3().inverted().transposed()
        remove = []
        for face in bm.faces:
            zs = [(piece.matrix_world @ vertex.co).z for vertex in face.verts]
            assert min(zs) >= joint - 1e-3 or max(zs) <= joint + 1e-3
            is_lower = min(zs) < joint - 1e-3 or (
                max(zs) <= joint + 1e-3 and (normals @ face.normal).z > 0)
            if is_lower != lower:
                remove.append(face)
        bmesh.ops.delete(bm, geom=remove, context="FACES")
        bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=1e-5)
        bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
        assert bm.faces and all(edge.is_manifold for edge in bm.edges), "Wall shell is not closed"
        if lower:
            # Avoid a coplanar union at the internal interface. This overlap is
            # removed by the union; it does not expand the outer wall boundary.
            delta = piece.matrix_world.inverted().to_3x3() @ Vector((0, 0, 0.001))
            for vertex in bm.verts:
                if abs((piece.matrix_world @ vertex.co).z - joint) < 1e-3:
                    vertex.co += delta
        bm.to_mesh(piece.data)
        bm.free()
    bpy.context.view_layer.objects.active = obj
    union = obj.modifiers.new("Join wall shells", "BOOLEAN")
    union.operation = "UNION"
    union.solver = "MANIFOLD"
    union.object = upper
    bpy.ops.object.modifier_apply(modifier=union.name)
    bpy.data.objects.remove(upper, do_unlink=True)


roots = {name: bpy.data.objects.get(name) for name in config["nodes"]}
assert all(roots.values()), "Missing stable wall ownership nodes"
floor_root = roots[config["floorNode"]]
material_source = next(obj for obj in floor_root.children_recursive if obj.type == "MESH")
assert material_source.data.uv_layers.active, "The floor needs its owning wall's projected texture"
normal_matrix = material_source.matrix_world.to_3x3().inverted().transposed()
caps = [face for face in material_source.data.polygons
        if (normal_matrix @ face.normal).normalized().z > 0.9]
assert caps, "No upward wall cap for the provisional floor material"
cap = max(caps, key=lambda face: ((material_source.matrix_world @ face.center).z, face.area))
# The atlas has separate face charts. Use a cap texel for this geometric draft;
# full landing texturing and visual review remain required before publication.
floor_uv = [sum(material_source.data.uv_layers.active.data[i].uv[axis]
                for i in cap.loop_indices) / len(cap.loop_indices) for axis in range(2)]
floor_material = material_source.data.materials[0]
cutter = prism("Draft doorway cutter", config["cutter"])
changed = []
for root in roots.values():
    for obj in list(root.children_recursive):
        if obj.type != "MESH":
            continue
        before = len(obj.data.polygons)
        join_stacked_shells(obj)
        # GLB vertices split at UV seams; restore geometric adjacency before CSG.
        bm = bmesh.new()
        bm.from_mesh(obj.data)
        bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=1e-5)
        bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
        bm.to_mesh(obj.data)
        bm.free()
        bpy.context.view_layer.objects.active = obj
        modifier = obj.modifiers.new("Draft through doorway", "BOOLEAN")
        modifier.operation = "DIFFERENCE"
        modifier.solver = "MANIFOLD"
        modifier.object = cutter
        bpy.ops.object.modifier_apply(modifier=modifier.name)
        assert obj.data.polygons, f"Doorway removed the entire wall: {obj.name}"
        repaired = obj.data.validate(verbose=True)
        bm = bmesh.new()
        bm.from_mesh(obj.data)
        # Preserve the Boolean's topology and choose final diagonals here,
        # before the GLB exporter processes material and texture seams.
        bmesh.ops.triangulate(bm, faces=list(bm.faces))
        bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
        assert all(len(face.verts) == 3 for face in bm.faces)
        minimum_area = min(face.calc_area() for face in bm.faces)
        degenerate = sum(face.calc_area() <= 0 for face in bm.faces)
        small_faces = sum(face.calc_area() < 1e-8 for face in bm.faces)
        non_manifold = sum(not edge.is_manifold for edge in bm.edges)
        if degenerate or non_manifold:
            (config_path.parent / "mesh-failure.json").write_text(json.dumps({
                "object": obj.name, "nonManifoldEdges": non_manifold,
                "degenerateFaces": [{"area": face.calc_area(),
                    "vertices": [list(obj.matrix_world @ vertex.co) for vertex in face.verts]}
                    for face in bm.faces if face.calc_area() < 1e-8],
            }))
        bm.to_mesh(obj.data)
        bm.free()
        assert not degenerate, f"Degenerate doorway faces: {obj.name}"
        assert not non_manifold, f"Open doorway wall mesh: {obj.name} ({non_manifold} edges)"
        changed.append({"object": obj.name, "beforeFaces": before,
                        "afterFaces": len(obj.data.polygons), "meshValidationRepaired": repaired,
                        "nonManifoldEdges": non_manifold, "smallFaces": small_faces,
                        "minimumFaceArea": minimum_area})
bpy.data.objects.remove(cutter, do_unlink=True)
floor = prism("Draft doorway floor", config["floor"])
floor.parent = floor_root
floor.matrix_parent_inverse = floor_root.matrix_world.inverted()
floor.data.materials.append(floor_material)
uv = floor.data.uv_layers.new(name="UVMap")
for loop in floor.data.loops:
    uv.data[loop.index].uv = floor_uv
bpy.ops.export_scene.gltf(filepath=str(output), export_format="GLB", export_extras=True)
report = {"scope": config["scope"], "published": False, "changed": changed,
          "floorMaterialStatus": "provisional source-cap texel; surface texturing pending",
          "modelSha256": hashlib.sha256(output.read_bytes()).hexdigest()}
(config_path.parent / "mesh-build.json").write_text(json.dumps(report))
print(json.dumps(report))
