"""Replace the visible cart window paint with a real framed shell opening."""
import sys, json, math
from pathlib import Path
import bpy, bmesh
from mathutils import Vector
from mathutils.bvhtree import BVHTree
HERE = Path(__file__).resolve().parent
sys.path[:0] = [str(HERE), str(HERE.parents[1] / 'refinement'), str(HERE.parents[1] / 'refinement/blender')]
from catalog import OUT
from tree_geometry import RAY, SIN, COS
from scenery_geometry import Mesh
from log_trap_state_candidate import point
from evidence_io import sha, write_json
from render_slots import acquire, release


def main():
    source = OUT / 'restart2-state/south-cart-wreck-solid-v3'; prior = json.loads((source / 'manifest.json').read_text())
    assert sha(source / 'worker.blend') == prior['model_sha256']
    dest = OUT / 'restart2-state/south-cart-window-v4'; dest.mkdir(exist_ok=False)
    bpy.ops.wm.open_mainfile(filepath=str(source / 'worker.blend')); scene = bpy.context.scene
    roof = scene.objects['Tipped barrel canopy shell']; assert roof.matrix_world.is_identity
    tree = BVHTree.FromPolygons([v.co for v in roof.data.vertices], [p.vertices[:] for p in roof.data.polygons])
    inner = [(94, 31), (103, 37), (96, 45), (85, 39)]
    outer = [(95, 28), (106, 36), (97, 49), (81, 40)]
    def hit(pixel):
        p = point(953 + pixel[0], 844 + pixel[1], 0)
        result = tree.ray_cast(p + RAY * 3000, -RAY, 6000)
        assert result[0] is not None, pixel
        return result[0]
    hits = [hit(p) for p in inner]; depth = sum(p.dot(RAY) for p in hits) / 4
    vertices = []
    for offset in [-12, 12]:
        for x, y in inner:
            p = point(953 + x, 844 + y, 0); vertices.append(p + RAY * (depth - p.dot(RAY) + offset))
    faces = [(3, 2, 1, 0), (4, 5, 6, 7), (0, 1, 5, 4), (1, 2, 6, 5), (2, 3, 7, 6), (3, 0, 4, 7)]
    data = bpy.data.meshes.new('Window cutting volume'); data.from_pydata(vertices, [], faces); data.update()
    cutter = bpy.data.objects.new(data.name, data); scene.collection.objects.link(cutter)
    bpy.context.view_layer.objects.active = roof; modifier = roof.modifiers.new('Real window opening', 'BOOLEAN'); modifier.operation = 'DIFFERENCE'; modifier.solver = 'EXACT'; modifier.object = cutter
    bpy.ops.object.modifier_apply(modifier=modifier.name); bpy.data.objects.remove(cutter, do_unlink=True)
    paint, gray = roof.data.materials[:2]
    def add(name, vertices, faces):
        data = bpy.data.meshes.new(name); data.from_pydata(vertices, [], faces); data.update()
        bm = bmesh.new(); bm.from_mesh(data); bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces)); bmesh.ops.triangulate(bm, faces=list(bm.faces))
        cuts = min(10, max(0, math.ceil(max(e.calc_length() for e in bm.edges) / 2) - 1))
        if cuts: bmesh.ops.subdivide_edges(bm, edges=list(bm.edges), cuts=cuts, use_grid_fill=True)
        bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces)); assert all(e.is_manifold for e in bm.edges); assert bm.calc_volume(signed=True) > 0
        bm.to_mesh(data); bm.free(); obj = bpy.data.objects.new(name, data); scene.collection.objects.link(obj); data.materials.append(paint); data.materials.append(gray)
        uv = data.uv_layers.new(name='Native target projection')
        for face in data.polygons:
            for loop in face.loop_indices:
                p = data.vertices[data.loops[loop].vertex_index].co; uv.data[loop].uv = ((p.x - 945) / 220, 1 - (-p.y * SIN - p.z * COS - 820) / 180)
        return obj
    # A closed wooden frame follows both observed diamond contours and the curved shell.
    outer_hits = [hit(p) for p in outer]; vertices = []
    for offset in [.8, -1.8]:
        for contour in [outer_hits, hits]: vertices.extend(p + RAY * offset for p in contour)
    faces = []
    for i in range(4):
        j = (i + 1) % 4
        faces.extend([(i, j, 4 + j, 4 + i), (8 + i, 12 + i, 12 + j, 8 + j), (i, 8 + i, 8 + j, j), (4 + i, 4 + j, 12 + j, 12 + i)])
    add('Window timber frame', vertices, faces)
    for n, pair in enumerate([[(95, 31), (96, 45)], [(85, 39), (103, 37)]]):
        a, b = [hit(p) + RAY * .2 for p in pair]; mesh = Mesh(); mesh.tube(a, b, .65, n=8); add(f'Window crossbar {n}', mesh.vertices, mesh.faces)
    objects = [o for o in scene.objects if o.type == 'MESH']; vertices = []; faces = []; audit = []
    for obj in objects:
        bm = bmesh.new(); bm.from_mesh(obj.data); assert all(e.is_manifold for e in bm.edges), obj.name
        volume = bm.calc_volume(signed=True); assert volume > 0, obj.name; bm.free()
        start = len(vertices); vertices.extend(v.co.copy() for v in obj.data.vertices); faces.extend(tuple(start + i for i in p.vertices) for p in obj.data.polygons)
        audit.append(dict(name=obj.name, closed_positive_volume=volume))
    bvh = BVHTree.FromPolygons(vertices, faces)
    for obj in objects:
        for face in obj.data.polygons:
            result = bvh.ray_cast(face.center + RAY * 2000, -RAY, 4000)
            face.material_index = 0 if face.normal.dot(RAY) > .05 and result[0] is not None and (result[0] - face.center).length < .05 else 1
    bpy.ops.wm.save_as_mainfile(filepath=str(dest / 'worker.blend'))
    camera = scene.camera; camera.data.ortho_scale = 240; center = Vector(tuple((min(p[i] for p in vertices) + max(p[i] for p in vertices)) / 2 for i in range(3)))
    acquire()
    try:
        for name, direction in [('00-native', RAY), ('01-reverse', Vector((.7, 1, .8)).normalized())]:
            target = point(1055, 910, 0) if name.startswith('00') else center
            camera.location = target + direction * 3000; camera.rotation_euler = (target - camera.location).to_track_quat('-Z', 'Y').to_euler()
            for mode in ['actual', 'solid']:
                scene.view_layers[0].material_override = gray if mode == 'solid' else None; scene.render.filepath = str(dest / f'{name}-{mode}.png'); bpy.ops.render.render(write_still=True)
    finally: release()
    write_json(dest / 'manifest.json', dict(status='Private framed opening, broken end panels still pending', model_sha256=sha(dest / 'worker.blend'), prior_model_sha256=prior['model_sha256'], inner_native_contour=inner, outer_native_contour=outer, components=audit, limitations=['Window depth and crossbar thickness inferred; source contours constrain visible aperture.', 'No appearance or complete wreck approval; broken panels, floor scraps and motion remain separate.']))


if __name__ == '__main__': main()
