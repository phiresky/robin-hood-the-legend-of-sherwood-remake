"""Review a bounded hall correction from native, isolated solid and neighboring contact cameras."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
BASE = ROOT / 'level-editor/work/york-refinement'
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('state', choices=['initial-initial', 'initial-applied', 'applied-initial', 'applied-applied'])
parser.add_argument('model', type=Path)
parser.add_argument('output', type=Path)
args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
if args.output.exists():
    raise FileExistsError(args.output)
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
from render_slots import acquire
acquire()
sys.path.insert(0, str(ROOT / 'level-editor/blender/nottingham'))
from freeze_tooling import select_tooling
select_tooling(json.loads((BASE / 'tooling/current.json').read_text())['directory'])
import bpy
from mathutils import Vector
from PIL import Image, ImageDraw
from render_views import render_views

sha = lambda p: hashlib.sha256(Path(p).read_bytes()).hexdigest()
digest = sha(args.model)
bpy.ops.wm.open_mainfile(filepath=str(args.model))
scene = bpy.data.scenes['york Refinement']
bpy.context.window.scene = scene
bpy.context.view_layer.update()
left, top, right, bottom = 2700, 410, 3055, 910
w, h = right-left, bottom-top
elevation = math.radians(35)
center = Vector(((left+right)/2, -(top+bottom)/2/math.sin(elevation), 0))
backward = Vector((0, -math.cos(elevation), math.sin(elevation)))
data = bpy.data.cameras.new('Native hall texture comparison')
camera = bpy.data.objects.new(data.name, data)
scene.collection.objects.link(camera)
data.type = 'ORTHO'
data.ortho_scale = max(w, h)
data.clip_start = .01
data.clip_end = 20000
camera.location = center+backward*10000
camera.rotation_euler = (-backward).to_track_quat('-Z', 'Y').to_euler()
scene.render.resolution_x = w*4
scene.render.resolution_y = h*4
render_views(scene.name, {'native': camera.name}, args.output, width=w*4, modes=('textured',))
first, second = args.state.split('-')
source = BASE/'restart2/hall-cover-source-combinations-v1'/f'patch001-{first}_patch002-{second}.png'
original = Image.open(source).convert('RGB').crop((left, top, right, bottom)).resize((w*4, h*4), Image.Resampling.NEAREST)
actual = Image.open(args.output/'native-textured.png').convert('RGB')
sheet = Image.new('RGB', (w*8, h*4+28), (32, 32, 32))
sheet.paste(original, (0, 28)); sheet.paste(actual, (w*4, 28))
draw = ImageDraw.Draw(sheet)
draw.text((8, 7), f'Original artwork: {args.state}', fill='white')
draw.text((w*4+8, 7), 'Saved texture candidate: native game camera', fill='white')
sheet.save(args.output/'original-native-comparison.png')
if digest != sha(args.model):
    raise ValueError('Native review changed saved model')
(args.output/'evidence.json').write_text(json.dumps({'state': args.state,
    'model_sha256': digest, 'source_sha256': sha(source), 'model_unchanged': True,
    'source_crop': [left, top, right, bottom], 'native_elevation_degrees': 35,
    'comparison_sha256': sha(args.output/'original-native-comparison.png')}, indent=2)+'\n')

from render_multiview_asset import render
sys.path.insert(0,str(Path(__file__).resolve().parent))
from restart2_camera_audit import labeled_copy
views=BASE/'restart2/hall-textures-v1'/args.state/'experiment/views.json'
manifest=json.loads(views.read_text());m=manifest['views'][0]['camera_matrix_world'];expected=((1,0,0),(0,math.sin(elevation),-math.cos(elevation)),(0,math.cos(elevation),math.sin(elevation)))
assert max(abs(m[r][c]-expected[r][c]) for r in range(3) for c in range(3))<1e-6
render(views,args.output/'solid',modes=('solid',),width=384)
from PIL import Image
for folder,mode in [(args.output/'solid','solid')]:
 tiles=[Image.open(folder/f'view-{i}-{mode}.png').convert('RGBA') for i in range(8)];sheet=Image.new('RGBA',(tiles[0].width*4,tiles[0].height*2))
 for i,tile in enumerate(tiles):sheet.paste(tile,((i%4)*tile.width,(i//4)*tile.height))
 sheet.save(folder/f'{mode}.png');labeled_copy(folder/f'{mode}.png',folder/f'{mode}-native-labeled.png')
context_nodes={'building-770','building-790','building-805','building-806','building-810','building-830','building-831'}
context=[o for o in scene.objects if o.type=='MESH' and not o.hide_render and o.get('source_node') in context_nodes]
original_groups={o:o.get('asset_group') for o in context}
for o in context:o['asset_group']=manifest['asset_id']
manifest['object_names']+= [o.name for o in context];manifest['render_object_names']=None
contact=args.output/'neighbor-contact';contact.mkdir();(contact/'views.json').write_text(json.dumps(manifest,indent=2)+'\n')
render(contact/'views.json',contact/'actual',width=384)
tiles=[Image.open(contact/f'actual/view-{i}-textured.png').convert('RGBA') for i in range(8)];sheet=Image.new('RGBA',(tiles[0].width*4,tiles[0].height*2))
for i,tile in enumerate(tiles):sheet.paste(tile,((i%4)*tile.width,(i//4)*tile.height))
sheet.save(contact/'textured.png');labeled_copy(contact/'textured.png',contact/'textured-native-labeled.png')
for o,g in original_groups.items():o['asset_group']=g
assert sha(args.model)==digest
(args.output/'control-review.json').write_text(json.dumps({'model_sha256':digest,'native_first':True,'neighbors':[o.name for o in context],'context_scope':'Existing scene neighbors at evaluated original transforms; source refreshed proxies are private diagnostics, not refined assets','geometry_and_file_unchanged':True,'status':'Awaiting visual review'},indent=2)+'\n')
