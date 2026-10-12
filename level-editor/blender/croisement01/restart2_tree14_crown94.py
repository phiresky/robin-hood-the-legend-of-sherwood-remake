"""Private source-constrained crown volume diagnostic; ownership remains provisional."""
import json
import shutil
import sys
from pathlib import Path

import bpy
from PIL import Image
from mathutils import Matrix

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT/'level-editor/refinement'), str(ROOT/'level-editor/refinement/blender'),
                str(ROOT/'level-editor/blender/leicester')]
from render_slots import acquire
from evidence_io import sha
from render_multiview_asset import render
from setup_map import fit_camera
import foliage_trees as foliage

acquire(slots=2)
assert shutil.disk_usage(ROOT).free > 10*1024**3 + 512*1024**2
assert int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines()
                if x.startswith('MemAvailable:')))*1024 > 6*1024**3
base = ROOT/'level-editor/work/croisement01-refinement/restart2'
wood = base/'tree14-wood-v13-projected/assets/croisement01-tree-14'
out = base/'tree14-crown94-geometry-v1'
assert not out.exists()
out.mkdir()
model = wood/'model.blend'
assert sha(model) == 'e8ddfe0bf595657b65ba982b3e0967434cb1bbf223f3c591fb623082f4f3cf2b'
bpy.ops.wm.open_mainfile(filepath=str(model))
config = json.loads((wood/'workspace.json').read_text())
scene = bpy.data.scenes[config['scene_name']]
bpy.context.window.scene = scene
wood_objects = [o for o in scene.objects if o.type == 'MESH' and o.get('asset_group') == config['asset_id']]
protected = {o.name: foliage.geometry_hash(o) for o in wood_objects}
# Seeds follow visible source clusters, not another tree's layout.
foliage.CONFIG[94] = dict(mask=94, ground=412., canopy_bottom=343,
    seeds=[(780,90),(825,130),(884,155),(725,165),(795,205),(915,215),(720,235),(850,285)])
evidence = foliage.source_packet(wood,94,out/'source-lobes')
for lobe in evidence['lobes']:
    lobe['depth_radius'] = 210.
mesh = bpy.data.meshes.new('Crown94 provisional volume')
crown = bpy.data.objects.new('Tree14 Crown94 provisional',mesh)
bpy.data.collections[config['collection_name']].objects.link(crown)
crown['asset_group'] = config['asset_id']
crown['source_node'] = 'foliage-tree14-crown94'
crown['projection_component'] = 'crown'
crown['ownership_status'] = 'PROVISIONAL mixed foliage and branch source, requires joint first-hit classification'
report = foliage.refine_crown(crown,94,evidence)
assert protected == {o.name: foliage.geometry_hash(o) for o in wood_objects}
points = [crown.matrix_world@v.co for v in crown.data.vertices]
extent = [max(p[i] for p in points)-min(p[i] for p in points) for i in range(3)]
assert extent[1] >= extent[0], extent
scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=256
saved = out/'model.blend'
bpy.ops.wm.save_as_mainfile(filepath=str(saved))
packet=json.loads((wood/'input/views.json').read_text())
packet['object_names']=[o.name for o in wood_objects]+[crown.name]
packet['render_object_names']=None
data=bpy.data.cameras.new('Joint crown diagnostic');data.type='ORTHO'
camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera)
for view in packet['views']:
    view['crop']={'width':384,'height':384}
    camera.matrix_world=Matrix(view['camera_matrix_world'])
    fit_camera(camera,wood_objects+[crown],1.,padding=1.2)
    view['camera_matrix_world']=[list(row) for row in camera.matrix_world]
    view['ortho_scale']=data.ortho_scale
manifest=out/'views.json';manifest.write_text(json.dumps(packet,indent=2)+'\n')
report.update(status='DIAGNOSTIC_NOT_READY_FOR_APPROVAL',model_sha256=sha(saved),
    wood_model_sha256=sha(model),wood_geometry_unchanged=True,extent_xyz=extent,
    limitations=['Crown94 association remains a joint source review hypothesis.',
      'Mixed native branches within leaf mask need actual first-hit ownership classification.',
      'Reverse and transverse neutral coverage and depth are inferred, not generated textures.'],
    texture_synthesis=False)
(out/'checkpoint.json').write_text(json.dumps(report,indent=2)+'\n')
render(manifest,out/'material-views',modes=('textured',),width=256)
images=[Image.open(out/f'material-views/view-{i}-textured.png').convert('RGB') for i in range(8)]
width,height=images[0].size;sheet=Image.new('RGB',(width*4,height*2))
for i,image in enumerate(images):sheet.paste(image,((i%4)*width,(i//4)*height))
sheet.save(out/'material-eight.png')
