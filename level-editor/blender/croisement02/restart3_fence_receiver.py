"""Apply the terminal cleared-fence artwork to an isolated real-ground state."""
import json
import math
from pathlib import Path
import sys

import bpy
import numpy as np
from PIL import Image, ImageDraw
from mathutils import Matrix, Vector

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(Path(__file__).parent), str(ROOT/'level-editor/refinement'),
               str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha, write_json
from render_slots import acquire, release
from restore_ground75_source import geometry
from review_bank_candidate import camera
from tree_geometry import SIN, COS, RAY

DEST = OUT/'restart3-fence-receiver/terminal-v2'
GROUND = OUT/'restart2-ground-completion/approved-fill-retry-v2/bake-v1/model.blend'
FENCE = OUT/'fence-state-candidate-v2/worker.blend'
PATCH = OUT/'source-states/mission-patches/mission-Emb05_FoB_MP-patch-022/transition-000.png'
GROUND_SHA = '16c638be71eeb76e86439a0fdb14bac1e7bb9562afe20d175b58d0df96fb4ec2'
PATCH_SHA = 'ec3b919539c3aa8fdfa4a43399479c48644a371c494bf0146128a20027fb2170'
BBOX = [1018, 811, 152, 152]


def atlas(obj):
    node = next(n for n in obj.data.materials[0].node_tree.nodes if n.type == 'TEX_IMAGE')
    values = np.empty(len(node.image.pixels), np.float32)
    node.image.pixels.foreach_get(values)
    return node, np.rint(values.reshape(1152,1792,4)[::-1]*255).astype('uint8')


def main():
    if DEST.exists():
        raise FileExistsError(DEST)
    if sha(GROUND) != GROUND_SHA or sha(PATCH) != PATCH_SHA:
        raise ValueError('Frozen receiver/source changed')
    layers = json.loads((OUT/'source-states/layers.json').read_text())
    rows = [r for r in layers['mission_patches'] if r['name'] == 'chariot02_barriere']
    if len(rows) != 3:
        raise ValueError('Fence mission replicas changed')
    for row in rows:
        if not row['state']['integrate_in_background'] or row['applied_graphic_mode'] != 'baked-last-transition-frame':
            raise ValueError('Terminal background semantics changed')
    patch = np.array(Image.open(PATCH).convert('RGBA'))
    if patch.shape != (152,152,4) or not (patch[:,:,3] == 255).all():
        raise ValueError('Full opaque terminal footprint changed')
    acquire()
    try:
        DEST.mkdir(parents=True)
        # Read evaluated source transforms before importing the contextual solids.
        bpy.ops.wm.open_mainfile(filepath=str(FENCE))
        bpy.context.view_layer.update()
        names = json.loads((OUT/'fence-state-candidate-v2/applied-views.json').read_text())['object_names']
        transforms = {n: [list(r) for r in bpy.data.objects[n].matrix_world] for n in names}
        signatures = {n: geometry(bpy.data.objects[n]) for n in names}
        fence_hash = sha(FENCE)
        bpy.ops.wm.open_mainfile(filepath=str(GROUND))
        bpy.context.preferences.filepaths.save_version = 0
        obj = bpy.data.objects['Croisement02 Terrain']
        bpy.context.view_layer.update()
        original_geometry = geometry(obj)
        node, original = atlas(obj)
        expected = original.copy()
        x,y,w,h = BBOX
        expected[y:y+h,x:x+w] = patch
        domain = np.zeros((1152,1792),bool)
        domain[y:y+h,x:x+w] = True
        if not np.array_equal(original[~domain],expected[~domain]):
            raise ValueError('Outside terminal footprint changed')
        Image.fromarray(original).save(DEST/'base-atlas.png')
        Image.fromarray(expected).save(DEST/'applied-atlas.png')
        Image.fromarray(domain.astype('uint8')*255).save(DEST/'domain.png')
        Image.fromarray(patch).save(DEST/'terminal-source.png')
        image = bpy.data.images.load(str(DEST/'applied-atlas.png'),check_existing=False)
        image.pack()
        node.image = image
        bpy.ops.wm.save_as_mainfile(filepath=str(DEST/'model.blend'),compress=True)
        bpy.ops.wm.open_mainfile(filepath=str(DEST/'model.blend'))
        obj = bpy.data.objects['Croisement02 Terrain']
        bpy.context.view_layer.update()
        if geometry(obj) != original_geometry or not np.array_equal(atlas(obj)[1],expected):
            raise ValueError('Reopened geometry/UV or packed artwork differs')
        ground_vertices = [obj.matrix_world@v.co for v in obj.data.vertices]
        if any(abs(v.z)>0.001 for v in ground_vertices):
            raise ValueError('Expected approved planar receiver')
        scene = bpy.data.scenes.new('Cleared fence applied receiver review')
        bpy.context.window.scene = scene
        scene.collection.objects.link(obj)
        parent = obj.parent
        while parent is not None:
            if parent.name not in scene.objects:
                scene.collection.objects.link(parent)
            parent = parent.parent
        bpy.context.view_layer.update()
        if geometry(obj) != original_geometry:
            raise ValueError('Ground transform changed in contact scene')
        obj.hide_render = False
        with bpy.data.libraries.load(str(FENCE),link=False) as (source,target):
            target.objects = names
        for imported in target.objects:
            scene.collection.objects.link(imported)
        bpy.context.view_layer.update()
        for imported in target.objects:
            imported.parent = None
            imported.matrix_world = Matrix(transforms[imported.name])
            imported.hide_render = False
        bpy.context.view_layer.update()
        for imported in target.objects:
            if geometry(imported) != signatures[imported.name]:
                raise ValueError('Imported fence transform/geometry differs')
        contacts = []
        for imported in target.objects:
            verts = [imported.matrix_world@v.co for v in imported.data.vertices]
            contacts.append(dict(object=imported.name,minimum_z=min(v.z for v in verts),
                                 maximum_z=max(v.z for v in verts),ground_z=0))
        bpy.ops.wm.save_as_mainfile(filepath=str(DEST/'contact.blend'),compress=True)
        target_point = Vector((1067,-868/SIN,20))
        paths = []
        for index in range(8):
            angle = index*math.pi/4
            direction = Vector(RAY) if index == 0 else Vector((math.sin(angle)*COS,-math.cos(angle)*COS,SIN))
            camera(scene,target_point,direction,640,480,700)
            scene.render.filepath = str(DEST/f'view-{index}.png')
            bpy.ops.render.render(write_still=True,scene=scene.name)
            paths.append(Path(scene.render.filepath))
        sheet = Image.new('RGB',(1280,480),'#303030')
        for i,path in enumerate(paths):
            im = Image.open(path).convert('RGBA'); bg = Image.new('RGBA',im.size,'#303030');bg.alpha_composite(im)
            sheet.paste(bg.convert('RGB').resize((320,240)),(i%4*320,i//4*240))
        sheet.save(DEST/'actual8.png')
        crop = (900,730,1250,1000)
        before = Image.fromarray(original).crop(crop).convert('RGB')
        after = Image.fromarray(expected).crop(crop).convert('RGB')
        compare = Image.new('RGB',(700,300),'#202020')
        compare.paste(before,(0,30));compare.paste(after,(350,30))
        draw = ImageDraw.Draw(compare);draw.text((8,8),'Approved base-state ground',fill='white');draw.text((358,8),'Applied-state terminal receiver',fill='white')
        compare.save(DEST/'base-applied-source.png')
        if sha(GROUND) != GROUND_SHA or sha(FENCE) != fence_hash or sha(PATCH) != PATCH_SHA:
            raise ValueError('Source changed during review')
        write_json(DEST/'validation.json',dict(status='PASS guarded applied-state receiver; visual review pending',
            model_sha256=sha(DEST/'model.blend'),source_model_sha256=GROUND_SHA,source_patch_sha256=PATCH_SHA,
            fence_model_sha256=fence_hash,context_sha256=sha(DEST/'contact.blend'),
            geometry_uv_signature=original_geometry,geometry_uv_unchanged=True,
            full_source_pixels=23104,outside_pixels_preserved=int((~domain).sum()),outside_changed=0,
            packed_rgba_exact=True,source_camera_first=True,receiver_nominal_z=0,receiver_world_z_range=[min(v.z for v in ground_vertices),max(v.z for v in ground_vertices)],contacts=contacts,
            bbox=BBOX,mission_replicas=[r['id'] for r in rows],
            behavior='Separate applied-state ground only: retain the last transition artwork while applied; base state keeps its approved atlas.',
            limitations=['Cleared fence geometry is approved; new cut-end textures remain pending.',
                         'This receiver is a private derivative, not an installed runtime state or a new permanent ground ownership assignment.'],
            user_approval=None,publication=False,
            evidence={p.name:sha(p) for p in DEST.glob('*.png')}))
    finally:
        release()


if __name__ == '__main__':
    main()
