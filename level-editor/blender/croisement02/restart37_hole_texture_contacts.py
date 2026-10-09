"""Review newly filled hole materials using the retained private contact geometry."""
import sys, json, math, shutil
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
from mathutils import Vector

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from restart5_initial_net_candidate_v2 import point
from tree_geometry import RAY, SIN, COS
from catalog import OUT
from evidence_io import sha, write_json
from render_slots import acquire, release

ROOT = OUT / 'restart9-hole-endpoints'
PINS = {
    'initial': ('candidate-v2/initial', '24c2c300bf9e940225676c2dbc4f5529865af1f6564bce6b0f1aa77ed299db4f'),
    'applied': ('candidate-v3/applied', 'b6528505a80f19e0b2aef610cfa4915603a001361a291e636ed9bf3b7c2e6f5d'),
}


def clip_halfplane(poly, a, b, inside=True):
    result = []
    sign = 1 if inside else -1
    def distance(p):
        return sign*((b[0]-a[0])*(p[1]-a[1])-(b[1]-a[1])*(p[0]-a[0]))
    for p, q in zip(poly, poly[1:]+poly[:1]):
        dp, dq = distance(p), distance(q)
        if dp >= -1e-9:
            result.append(p)
        if (dp > 1e-9 and dq < -1e-9) or (dp < -1e-9 and dq > 1e-9):
            t = dp/(dp-dq)
            result.append([p[i]+t*(q[i]-p[i]) for i in range(2)])
    return result


def clipped_pieces(projected, rectangle, aperture, applied):
    current = projected.tolist()
    for a, b in zip(rectangle, rectangle[1:]+rectangle[:1]):
        current = clip_halfplane(current, a, b)
        if len(current) < 3:
            return []
    if not applied:
        return [current]
    pieces = []
    # Disjoint outside pieces of a convex aperture retain exact triangle UVs.
    for a, b in zip(aperture, aperture[1:]+aperture[:1]):
        outside = clip_halfplane(current, a, b, inside=False)
        if len(outside) >= 3:
            pieces.append(outside)
        current = clip_halfplane(current, a, b)
        if len(current) < 3:
            break
    return pieces


def main(terrain, phase):
    assert shutil.disk_usage(OUT).free > 25 * 1024**3
    model = OUT / 'restart25-approved-state-materialization-v1/official-texture-experiments-v2' / ('hole-' + phase) / 'experiment/native-state-v1/worker.blend'
    proof = json.loads(model.with_name('preservation.json').read_text())
    digest = proof['model_sha256']
    assert sha(model) == digest
    audit = json.loads((ROOT / 'receiver-audit-v2/report.json').read_text())
    anchor = [1590, 767] if terrain == 'flat' else [688, 177]
    row = next(r for r in audit['positions'] if r['display_position'] == anchor)
    z = 0. if terrain == 'flat' else row['center']['hit'][2]
    parent = next(r for r in audit['models'] if ('ground-candidate' if terrain == 'flat' else 'bank103') in r['path'])
    assert sha(Path(parent['path'])) == parent['sha256']
    dest = model.parent / ('contact-' + terrain)
    dest.mkdir(parents=True, exist_ok=False)
    bpy.ops.wm.open_mainfile(filepath=str(model))
    scene = bpy.context.scene
    endpoint = [o for o in scene.objects if o.type == 'MESH']
    assert len(endpoint) == 1
    endpoint[0].location = point(anchor[0], anchor[1], z)
    with bpy.data.libraries.load(parent['path'], link=False) as (data, loaded):
        loaded.objects = [r['name'] for r in parent['objects']]
    sources = list(loaded.objects)
    for ob in sources:
        scene.collection.objects.link(ob)
    bpy.context.view_layer.update()
    for ob in sources:
        saved = next(r for r in parent['objects'] if r['name'] == ob.name)
        assert np.max(np.abs(np.array(ob.matrix_world) - np.array(saved['matrix']))) < 1e-7

    crop = [anchor[0]-10, anchor[1]+10, anchor[0]+110, anchor[1]+110]
    rectangle = [[crop[0],crop[1]],[crop[2],crop[1]],[crop[2],crop[3]],[crop[0],crop[3]]]
    aperture = [(anchor[0]+49+17*math.cos(t), anchor[1]+56+12*math.sin(t))
                for t in np.linspace(0, math.tau, 97)[:-1]]
    vertices, faces, material_indices, uv_values, materials = [], [], [], {}, []
    source_triangles = 0
    for ob in sources:
        mesh = ob.data
        mesh.calc_loop_triangles()
        base_mat = len(materials)
        materials.extend(list(mesh.materials))
        for layer in mesh.uv_layers:
            uv_values.setdefault(layer.name, [(0., 0.)] * len(vertices))
        for tri in mesh.loop_triangles:
            world = [ob.matrix_world @ mesh.vertices[i].co for i in tri.vertices]
            if max(abs(v.z-z) for v in world) > .02:
                continue
            projected = np.array([[v.x, -v.y*SIN-v.z*COS] for v in world])
            matrix = np.vstack([projected.T, np.ones(3)])
            if abs(np.linalg.det(matrix)) < 1e-8:
                continue
            pieces = clipped_pieces(projected, rectangle, aperture, phase == 'applied')
            inverse = np.linalg.inv(matrix)
            for piece in pieces:
                for j in range(1,len(piece)-1):
                    cut = [piece[0],piece[j],piece[j+1]]
                    if abs(np.linalg.det(np.vstack([np.array(cut).T,np.ones(3)]))) < 1e-8:
                        continue
                    ids = []
                    for x, y in cut:
                        weights = inverse @ np.array([x, y, 1.])
                        position = sum((world[i]*float(weights[i]) for i in range(3)), Vector())
                        ids.append(len(vertices))
                        vertices.append(tuple(position))
                        for name in uv_values:
                            layer = mesh.uv_layers.get(name)
                            value = tuple(sum((layer.data[tri.loops[i]].uv * float(weights[i]) for i in range(3)), Vector((0., 0.)))) if layer else (0., 0.)
                            uv_values[name].append(value)
                    faces.append(tuple(ids))
                    material_indices.append(base_mat + tri.material_index)
            source_triangles += 1
        ob.hide_render = True
    assert faces
    mesh = bpy.data.meshes.new('Private local receiver aperture ' + phase)
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    local = bpy.data.objects.new(mesh.name, mesh)
    scene.collection.objects.link(local)
    for mat in materials:
        mesh.materials.append(mat)
    for i, poly in enumerate(mesh.polygons):
        poly.material_index = material_indices[i]
    for name, values in uv_values.items():
        layer = mesh.uv_layers.new(name=name)
        for loop in mesh.loops:
            layer.data[loop.index].uv = values[loop.vertex_index]
    local['private_state_local_receiver'] = True
    local['approved_parent_unchanged'] = parent['sha256']
    bpy.context.view_layer.update()
    cam = scene.camera
    center = point(anchor[0]+50, anchor[1]+58, z)
    cam.data.ortho_scale = 190
    scene.render.resolution_x = 384
    scene.render.resolution_y = 384
    cameras = []
    for i in range(8):
        angle = -math.pi/2+i*math.pi/4
        direction = Vector((math.cos(angle)*COS, math.sin(angle)*COS, SIN))
        cam.location = center+direction*3000
        cam.rotation_euler = (center-cam.location).to_track_quat('-Z', 'Y').to_euler()
        bpy.context.view_layer.update()
        cameras.append(dict(view=i, matrix=[list(r) for r in cam.matrix_world], native_first=i == 0))
        scene.render.filepath = str(dest/f'view-{i}.png')
        bpy.ops.render.render(write_still=True)
    sheet = Image.new('RGB', (1536, 768), (30, 30, 30))
    for i in range(8):
        im = Image.open(dest/f'view-{i}.png').convert('RGBA')
        sheet.paste(im, ((i % 4)*384, (i//4)*384), im)
    sheet.save(dest/'contact8.png')
    target = point((crop[0]+crop[2])/2, (crop[1]+crop[3])/2, z)
    cam.location = target+RAY*3000
    cam.rotation_euler = (target-cam.location).to_track_quat('-Z', 'Y').to_euler()
    cam.data.ortho_scale = crop[2]-crop[0]
    scene.render.resolution_x = 600
    scene.render.resolution_y = 500
    scene.render.filepath = str(dest/'native.png')
    bpy.ops.render.render(write_still=True)
    plan = json.loads((ROOT/'source-plan-v1/plan.json').read_text())
    frame = next(r for r in plan['phases'] if r['phase'] == phase)
    expected = Image.open(OUT/'source-states/covered.png').convert('RGBA')
    expected.alpha_composite(Image.open(frame['source']).convert('RGBA'),
                             tuple(anchor[i]+frame['offset'][i] for i in range(2)))
    expected = expected.crop(crop).resize((600, 500), Image.Resampling.NEAREST)
    observed = Image.open(dest/'native.png').convert('RGBA')
    comp = Image.new('RGB', (1200, 500), (35, 35, 35))
    comp.paste(expected, (0, 0), expected)
    comp.paste(observed, (600, 0), observed)
    comp.save(dest/'source-comparison.png')
    write_json(dest/'report.json', dict(model=str(model), model_sha256=digest,
        receiver_parent=parent, display_position=anchor, support_z=z,
        endpoint_translation=list(endpoint[0].location), local_receiver_triangles=len(faces),
        source_receiver_triangles=source_triangles, aperture_source_polygon=aperture,
        crop=crop, cameras=cameras, approved_parent_unchanged=sha(Path(parent['path'])) == parent['sha256'],
        status='Private state-local aperture hypothesis; no approved terrain, runtime, or bindings changed',
        limitation='State delivery must replace only this local receiver region while applied and restore it on reset. This is material/contact review, not runtime integration proof. Private crop perimeter is review framing, not a proposed map cut.'))


if __name__ == '__main__':
    acquire()
    try:
        args = sys.argv[sys.argv.index('--')+1:]
        main(*args)
    finally:
        release()
