"""Add separately inferred native boundary specks without changing shrub93."""
import json
import sys
from pathlib import Path
import bpy
import numpy as np
from mathutils import Vector
from PIL import Image

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(Path(__file__).parent), str(ROOT/'level-editor/refinement'),
               str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT, scenery_workspace
from evidence_io import sha, write_json
from refinement_workspace import _geometry
from render_slots import acquire, release
from tree_geometry import SIN, COS, RAY, material, one_sided
from render_multiview_asset import render


def main():
    asset = 'croisement02-shrub-93'
    original = scenery_workspace(asset)
    digest = sha(original/'model.blend')
    destination = OUT/'restart2-vegetation/shrub93-boundary-v2'
    destination.mkdir(exist_ok=False, parents=True)
    boundary_path = OUT/'mixed-wood-audit/boundary-roles76-93-v1/93-foliage93.png'
    boundary = np.asarray(Image.open(boundary_path).convert('L')) > 0
    known_path = OUT/'mixed-wood-audit/domain-503.png'
    known = np.asarray(Image.open(known_path).convert('L')) > 0
    assert boundary.sum() == 25 and not np.any(boundary & known)
    source = np.asarray(Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA')).copy()
    source[:, :, 3] = boundary*255
    texture = destination/'inferred-boundary-rgb.png'
    Image.fromarray(source).save(texture)
    bpy.ops.wm.open_mainfile(filepath=str(original/'model.blend'))
    bpy.context.preferences.filepaths.save_version = 0
    collection = bpy.data.collections['Croisement02 Working']
    old = [o for o in collection.all_objects if o.type == 'MESH' and o.get('asset_group') == asset]
    assert len(old) == 3
    before = {o.name: _geometry(o, protect_appearance=True) for o in old}
    prior = old[0]
    points = []
    for original_object in old:
        indices = sorted({v for p in original_object.data.polygons
                          if original_object.data.materials[p.material_index].get('foliage_observed')
                          for v in p.vertices})
        points.extend(tuple(original_object.matrix_world@original_object.data.vertices[i].co)
                      for i in indices)
    points = np.asarray(points)
    screen = np.column_stack((points[:, 0], -points[:, 1]*SIN-points[:, 2]*COS))
    vertices, faces, uvs, records = [], [], [], []
    right = np.array([1., 0., 0.]); down = np.array([0., -SIN, -COS]); ray = np.asarray(RAY)
    for sy, sx in zip(*np.nonzero(boundary)):
        target = np.array([sx+.5, sy+.5])
        nearest = int(np.argmin(np.sum((screen-target)**2, axis=1)))
        center = points[nearest]+right*(target[0]-screen[nearest, 0])+down*(target[1]-screen[nearest, 1])
        corners = [center+right*dx+down*dy for dx, dy in [(-.5, -.5), (.5, -.5), (.5, .5), (-.5, .5)]]
        if np.dot(np.cross(corners[1]-corners[0], corners[2]-corners[0]), ray) < 0:
            corners.reverse()
        for back in (False, True):
            points_on_face = [p-ray*.02 for p in reversed(corners)] if back else corners
            start = len(vertices)
            vertices.extend(tuple(p) for p in points_on_face)
            faces.append(tuple(range(start, start+4)))
            uvs.extend((p[0]/1792, 1-(-p[1]*SIN-p[2]*COS)/1152) for p in points_on_face)
        records.append(dict(source_pixel=[int(sx), int(sy)], center=center.tolist(),
                            nearest_observed_leaf_distance_pixels=float(np.linalg.norm(screen[nearest]-target)),
                            source_role='Contextually inferred boundary foliage; not observed domain503'))
    mesh = bpy.data.meshes.new('Shrub93 inferred boundary fragments')
    mesh.from_pydata(vertices, [], faces); mesh.update()
    obj = bpy.data.objects.new(mesh.name, mesh); collection.objects.link(obj)
    for key in ['asset_group', 'asset_name', 'source_node', 'part_name']:
        obj[key] = prior[key]
    obj['projection_component'] = 'crown'
    obj['inferred_boundary_domain'] = 6003
    added_name = obj.name
    mat = material('Shrub93 inferred native boundary fronts and backs', texture, False)
    mat['texture_provenance'] = 'Native RGB with inferred receiver role; separate25 boundary pixels, not observed503'
    one_sided(mat); mesh.materials.append(mat)
    uv = mesh.uv_layers.new(name='Foliage UV')
    ownership = mesh.color_attributes.new(name='Source ownership', type='FLOAT_COLOR', domain='CORNER')
    mesh.color_attributes.active_color = ownership
    for i, coords in enumerate(uvs):
        uv.data[i].uv = coords; ownership.data[i].color = (0., 1., 1., 1.)
    assert before == {o.name: _geometry(o, protect_appearance=True) for o in old}
    bpy.ops.wm.save_as_mainfile(filepath=str(destination/'model.blend'), compress=True)
    output_hash = sha(destination/'model.blend')
    bpy.ops.wm.open_mainfile(filepath=str(destination/'model.blend'))
    assert before == {name: _geometry(bpy.data.objects[name], protect_appearance=True) for name in before}
    assert sha(original/'model.blend') == digest
    write_json(destination/'preservation.json', dict(status='PASS immutable original geometry/UV/materials',
               original_worker=str(original), original_model_sha256=digest, model_sha256=output_hash,
               known_domain_sha256=sha(known_path), boundary_sha256=sha(boundary_path),
               original_objects=list(before), added_object=added_name, inferred_boundary_pixels=25,
               additions=records, limitations=['Nearest own observed leaf depth is inferred.',
                   'Paired one-pixel leaf fragments do not assert observed thickness or new trunk geometry.',
                   'Original domain503 and all original packed textures remain unchanged.',
                   'No canonical selection, user approval, or publication implied.']))
    scene = bpy.data.scenes['Croisement02 Refinement']
    scene.render.engine = 'CYCLES'; scene.cycles.samples = 4; scene.cycles.transparent_max_bounces = 256
    packet = json.loads((original/'modified/views.json').read_text())
    for field in ('object_names', 'render_object_names'):
        if packet.get(field) is not None: packet[field].append(added_name)
    for view in packet['views']:
        view['crop'] = {'width': packet['tile_size'][0], 'height': packet['tile_size'][1]}
    write_json(destination/'cameras.json', packet)
    render(destination/'cameras.json', destination/'actual', width=384)
    images = [Image.open(destination/f'actual/view-{i}-textured.png').convert('RGB') for i in range(8)]
    w, h = images[0].size; sheet = Image.new('RGB', (w*4, h*2))
    for i, image in enumerate(images): sheet.paste(image, ((i%4)*w, (i//4)*h))
    sheet.save(destination/'actual/sheet.png')
    assert sha(destination/'model.blend') == output_hash
    print(destination)


if __name__ == '__main__':
    acquire()
    try: main()
    finally: release()
