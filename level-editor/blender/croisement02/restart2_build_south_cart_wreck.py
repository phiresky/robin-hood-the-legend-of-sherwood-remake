"""Reopenable private whole-cart hypothesis with native-first diagnostic views."""
import json, math, sys
from pathlib import Path
import bpy, bmesh, numpy as np
from PIL import Image, ImageDraw
from mathutils import Vector
from mathutils.bvhtree import BVHTree
HERE = Path(__file__).resolve().parent
sys.path[:0] = [str(HERE), str(HERE.parents[1] / 'refinement'), str(HERE.parents[1] / 'refinement/blender')]
from catalog import OUT
from scenery_geometry import Mesh
from tree_geometry import SIN, COS, RAY
from log_trap_state_candidate import point, material
from evidence_io import sha, write_json
from render_slots import acquire, release


def main():
    fitpath = OUT / 'restart2-state/south-cart-body-fit-v2/fit.json'
    fit = json.loads(fitpath.read_text())
    dest = OUT / 'restart2-state/south-cart-wreck-solid-v1'; dest.mkdir(exist_ok=False)
    manifest = json.loads((OUT / 'state-target-evidence/south-cart/manifest.json').read_text())
    box = [945, 820, 1165, 1000]; source = Image.new('RGBA', (220, 180))
    for index in [0, 1]:
        part = manifest['parts'][index]; frame = part['frames'][-1]; path = Path(frame['image']); assert sha(path) == frame['image_sha256']
        rgba = np.array(Image.open(path).convert('RGBA')); domain = Image.new('L', (rgba.shape[1], rgba.shape[0]))
        if index == 0: ImageDraw.Draw(domain).polygon(fit['body_domain_polygon'], fill=255)
        else: ImageDraw.Draw(domain).rectangle((250, 0, rgba.shape[1], rgba.shape[0]), fill=255)
        rgba[:, :, 3] = np.minimum(rgba[:, :, 3], np.asarray(domain))
        rgba[np.all(rgba[:, :, :3] == [0, 0, 255], axis=2), 3] = 0
        x, y = [int(part['position'][i] + frame['offset'][i]) for i in range(2)]
        source.alpha_composite(Image.fromarray(rgba), (x - box[0], y - box[1]))
    source.save(dest / 'bounded-source.png')
    bpy.ops.wm.read_factory_settings(use_empty=True); scene = bpy.context.scene
    scene.render.engine = 'CYCLES'; scene.cycles.samples = 8; scene.cycles.use_denoising = False
    scene.view_settings.view_transform = 'Standard'; scene.render.film_transparent = True
    scene.render.image_settings.color_mode = 'RGBA'; scene.render.resolution_x = scene.render.resolution_y = 512
    scene.world = bpy.data.worlds.new('World'); scene.world.use_nodes = True
    scene.world.node_tree.nodes['Background'].inputs[0].default_value = (.15, .15, .15, 1)
    paint = material(dest / 'bounded-source.png'); gray = bpy.data.materials.new('Unobserved wreck structure'); gray.use_nodes = True
    gray.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value = (.17, .17, .17, 1)
    pieces = list(fit['pieces']); placement = fit['placement']; origin = Vector(placement['origin']); across = Vector(placement['across']); up = Vector(placement['up'])
    along = Vector((61, -21 / SIN, 0)).normalized(); halfwidth = fit['parameters'][1]
    for axle in [0, placement['wheelbase']]:
        m = Mesh(); m.tube(origin + along * axle - across * halfwidth, origin + along * axle + across * halfwidth, 2, n=12)
        pieces.append(dict(name=f'Full axle {axle}', vertices=m.vertices, faces=m.faces))
        for side in [-1, 1]:
            center = origin + along * axle + across * (side * halfwidth)
            m = Mesh(); m.tube(center - across * 1.5, center + across * 1.5, 21, n=32)
            pieces.append(dict(name=f'Wheel {axle} {side}', vertices=m.vertices, faces=m.faces))
            m = Mesh(); m.tube(center - across * 3, center + across * 3, 4, n=12)
            pieces.append(dict(name=f'Hub {axle} {side}', vertices=m.vertices, faces=m.faces))
    objects = []; audits = []
    for piece in pieces:
        data = bpy.data.meshes.new(piece['name']); data.from_pydata(piece['vertices'], [], piece['faces']); data.update()
        bm = bmesh.new(); bm.from_mesh(data); bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
        assert all(e.is_manifold for e in bm.edges), piece['name']
        volume = bm.calc_volume(signed=True); assert volume > 0, piece['name']
        cuts = min(20, max(0, math.ceil(max(e.calc_length() for e in bm.edges) / 5) - 1))
        if cuts: bmesh.ops.subdivide_edges(bm, edges=list(bm.edges), cuts=cuts, use_grid_fill=True)
        bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces)); assert all(e.is_manifold for e in bm.edges)
        bm.to_mesh(data); bm.free(); data.update()
        obj = bpy.data.objects.new(piece['name'], data); scene.collection.objects.link(obj); objects.append(obj)
        obj['state'] = 'south cart terminal hypothesis'; obj['geometry_status'] = 'private unapproved coherent rigid template'
        data.materials.append(paint); data.materials.append(gray); uv = data.uv_layers.new(name='Native target projection')
        for face in data.polygons:
            for loop in face.loop_indices:
                p = data.vertices[data.loops[loop].vertex_index].co
                uv.data[loop].uv = ((p.x - box[0]) / 220, 1 - (-p.y * SIN - p.z * COS - box[1]) / 180)
        audits.append(dict(name=obj.name, positive_closed_volume=volume, minimum_z=min(v.co.z for v in data.vertices)))
    vertices = []; faces = []
    for obj in objects:
        start = len(vertices); vertices.extend(v.co.copy() for v in obj.data.vertices); faces.extend(tuple(start + i for i in p.vertices) for p in obj.data.polygons)
    bvh = BVHTree.FromPolygons(vertices, faces)
    for obj in objects:
        for face in obj.data.polygons:
            hit = bvh.ray_cast(face.center + RAY * 2000, -RAY, 4000)
            face.material_index = 0 if face.normal.dot(RAY) > .05 and hit[0] is not None and (hit[0] - face.center).length < .05 else 1
    sun = bpy.data.lights.new('Sun', 'SUN'); sun.energy = 2; light = bpy.data.objects.new('Sun', sun); scene.collection.objects.link(light); light.rotation_euler = (.6, -.5, -.4)
    data = bpy.data.cameras.new('Native-first cart diagnostic'); data.type = 'ORTHO'; data.clip_end = 10000
    camera = bpy.data.objects.new(data.name, data); scene.collection.objects.link(camera); scene.camera = camera
    bpy.ops.wm.save_as_mainfile(filepath=str(dest / 'worker.blend'))
    bounds = [v.co for o in objects for v in o.data.vertices]; center = Vector(tuple((min(p[i] for p in bounds) + max(p[i] for p in bounds)) / 2 for i in range(3)))
    acquire()
    try:
        for name, direction in [('00-native', RAY), ('01-reverse', Vector((.7, 1, .8)).normalized())]:
            target = point(1055, 910, 0) if name.startswith('00') else center
            camera.location = target + direction * 3000; camera.rotation_euler = (target - camera.location).to_track_quat('-Z', 'Y').to_euler(); data.ortho_scale = 240
            for mode in ['actual', 'solid']:
                scene.view_layers[0].material_override = gray if mode == 'solid' else None
                scene.render.filepath = str(dest / f'{name}-{mode}.png'); bpy.ops.render.render(write_still=True)
    finally: release()
    write_json(dest / 'manifest.json', dict(status='Private whole wreck diagnostic; no geometry approval', model_sha256=sha(dest / 'worker.blend'), fit_sha256=sha(fitpath), components=audits, source_first_hit_material_assignment=True, source_domain='Bounded body polygon and terminal wheel region; ground scraps, barrel, horses and fence excluded', limitations=['Rigid tipped template is a hypothesis; native broken roof, end boards and wheel identity still need visual assessment.', 'Native body survey missing1659 and excess893 pixels before wheel addition; not a final first-hit score.', 'Lower wheels and hidden boards inferred. Exact terrain contact remains unverified; finite wheel thickness can cross provisional Z0.', 'No physical motion, state completion, appearance generation or user approval.']))


if __name__ == '__main__': main()
