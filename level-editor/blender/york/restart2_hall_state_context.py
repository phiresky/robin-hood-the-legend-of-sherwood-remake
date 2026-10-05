"""Private native joint diagnostic with both castle covers removed exactly."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT/'level-editor/work/york-refinement'
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--version',default='hall-joint-revealed-v1')
parser.add_argument('--reproject-hall',action='store_true')
args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
destination = OUT/'restart2'/args.version
if destination.exists():
    raise FileExistsError(destination)
sys.path.insert(0, str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
sys.path.insert(0, str(ROOT/'level-editor/blender/nottingham'))
from freeze_tooling import select_tooling
select_tooling(json.loads((OUT/'tooling/current.json').read_text())['directory'])
import bpy
from mathutils import Vector
from source_projection_bake import bake
from render_views import render_views

source_model = OUT/'restart2/hall-room-v1/model.blend'
source_image = OUT/'restart2/hall-cover-source-combinations-v1/patch001-applied_patch002-applied.png'
bpy.ops.wm.open_mainfile(filepath=str(source_model))
scene = bpy.data.scenes['york Refinement']
bpy.context.window.scene = scene
bpy.context.view_layer.update()
working = bpy.data.collections['york Working']
objects = [o for o in working.all_objects if o.type=='MESH']
removed = [o for o in objects if o.get('source_node') in ('building-830','building-831','building-832')]
if len(removed)!=3:
    raise ValueError('Expected the exact three native cover obstacles')
groups = {o.get('asset_group') for o in removed}
if groups!={'york-castle-main-keep','york-castle-east-round-tower'}:
    raise ValueError('Unexpected cover ownership')
if args.reproject_hall:
    groups.add('york-castle-great-hall')
def shape(obj):
    return [list(obj.matrix_world@v.co) for v in obj.data.vertices], [list(f.vertices) for f in obj.data.polygons]
before = {o.name:shape(o) for o in objects}
def surface(obj):
    return ([[list(d.uv) for d in layer.data] for layer in obj.data.uv_layers],
            [m.name if m else None for m in obj.data.materials], obj.hide_render)
outside = {o.name:surface(o) for o in objects if o.get('asset_group') not in groups}
for obj in removed:
    obj.hide_render=True
destination.mkdir(parents=True)
for group in sorted(groups):
    nodes=sorted({o['source_node'] for o in objects if o.get('asset_group')==group and not o.hide_render})
    bake('york',source_image,destination/(group+'-projection.json'),
         receiver_nodes=nodes,receiver_asset_id=group,
         projection_label='private-native-cover-joint',texels_per_unit=2,preserve_authored=False)
if before!={o.name:shape(o) for o in objects}:
    raise ValueError('Context state projection changed world geometry')
if outside!={o.name:surface(o) for o in objects if o.get('asset_group') not in groups}:
    raise ValueError('Context state projection changed an unrelated receiver')
bpy.context.preferences.filepaths.save_version=0
bpy.ops.wm.save_as_mainfile(filepath=str(destination/'model.blend'),compress=True)
config=json.loads((OUT/'restart2/hall-room-v1/workspace.json').read_text())
config['source_path']=str(source_image)
(destination/'workspace.json').write_text(json.dumps(config,indent=2)+'\n')
scene.render.engine='CYCLES';scene.cycles.device='CPU';scene.cycles.samples=16
scene.cycles.use_denoising=False
scene.render.threads_mode='FIXED';scene.render.threads=2
scene.view_settings.view_transform='Standard';scene.view_settings.look='None'
scene.view_settings.exposure=0;scene.view_settings.gamma=1
left,top,right,bottom=2700,410,3055,910
w,h=right-left,bottom-top
elevation=math.radians(35)
center=Vector(((left+right)/2,-(top+bottom)/2/math.sin(elevation),0))
backward=Vector((0,-math.cos(elevation),math.sin(elevation)))
data=bpy.data.cameras.new('Native both-covers-applied diagnostic')
camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera)
data.type='ORTHO';data.ortho_scale=max(w,h);data.clip_start=.01;data.clip_end=20000
camera.location=center+backward*10000
camera.rotation_euler=(-backward).to_track_quat('-Z','Y').to_euler()
scene.render.resolution_x=w*4;scene.render.resolution_y=h*4
render_views(scene.name,{'native':camera.name},destination/'native-joint',width=w*4,modes=('textured',))
(destination/'evidence.json').write_text(json.dumps({
    'status':'HOLD: exact cover removal and context projection diagnostic, not complete state geometry',
    'source_model_sha256':hashlib.sha256(source_model.read_bytes()).hexdigest(),
    'source_image_sha256':hashlib.sha256(source_image.read_bytes()).hexdigest(),
    'removed_native_nodes':[o['source_node'] for o in removed],
    'projected_context_groups':sorted(groups),'world_geometry_preserved':True,
    'hall_geometry_unchanged':True,'hall_materials_unchanged':not args.reproject_hall,
    'camera':{'type':'ORTHO','native_elevation_degrees':35,'source_crop':[left,top,right,bottom]},
    'remaining':['Native mask646 arch has no linked obstacle: inspect whether dedicated geometry is needed.','Inspect east-tower fireplace receiver and geometry after native cover832 removal.','Separate patch001-only and patch002-only geometry conditions and candle states.']},indent=2)+'\n')
