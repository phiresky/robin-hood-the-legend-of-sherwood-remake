"""Join independent approved-geometry texture candidates for a native context review."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import sys
ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'level-editor/work/york-refinement'
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--version',default='assembled-v1')
parser.add_argument('--bay-bake',default='bake-v2-shadow')
parser.add_argument('--house-bake',default='bake-v1')
args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
OUT=BASE/'restart2/pair-textures-v1'/args.version
BAY='york-market-southeast-tall-narrow-house'
HOUSE='york-southwest-square-west-house'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
sys.path.insert(0,str(ROOT/'level-editor/blender/nottingham'))
from freeze_tooling import select_tooling
select_tooling(json.loads((BASE/'tooling/current.json').read_text())['directory'])
import bpy
from mathutils import Vector
from refinement_workspace import _geometry
from render_multiview_asset import render
from render_views import render_views
sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
bay=OUT.parent/BAY/args.bay_bake/'model.blend'
house=OUT.parent/HOUSE/args.house_bake/'model.blend'
for p in (bay,house):
    if sha(p)!=json.loads(p.with_name('validation.json').read_text())['baked_model_sha256']:
        raise ValueError('Bake model changed')
bpy.ops.wm.open_mainfile(filepath=str(bay))
scene=bpy.data.scenes['york Refinement'];bpy.context.window.scene=scene;bpy.context.view_layer.update()
before={o.name:_geometry(o) for o in scene.objects}
targets={o.name:o for o in scene.objects if o.type=='MESH' and not o.hide_render and o.get('asset_group')==HOUSE}
with bpy.data.libraries.load(str(house),link=False) as (available,imported):
    imported.objects=list(targets)
for name,donor in zip(targets,imported.objects):
    if donor.get('source_node')!=targets[name].get('source_node') or donor.get('asset_group')!=HOUSE:raise ValueError('Foreign donor')
    # Only local mesh data is copied; the reopened receiver keeps its evaluated transform.
    targets[name].data=donor.data.copy()
for donor in imported.objects:bpy.data.objects.remove(donor,do_unlink=True)
bpy.context.view_layer.update()
if before!={o.name:_geometry(o) for o in scene.objects}:raise ValueError('Assembly changed geometry')
OUT.mkdir();bpy.context.preferences.filepaths.save_version=0
bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'model.blend'),compress=True)
record={'status':'Private texture candidate, no user texture approval','geometry_preserved':True,
        'baked_model_sha256':sha(OUT/'model.blend'),'source_models':{str(p):sha(p) for p in (bay,house)}}
(OUT/'assembly.json').write_text(json.dumps(record,indent=2)+'\n')
groups={o.name:o.get('asset_group') for o in scene.objects if o.get('asset_group') in (BAY,HOUSE)}
for name in groups:scene.objects[name]['asset_group']='york-paired-house-inspection'
manifest=BASE/'restart2/pair-v16/assembled-review/pair/inspection-v1/actual-views.json'
render(manifest,OUT/'actual',width=384)
for name,group in groups.items():scene.objects[name]['asset_group']=group
crop=[550,1100,790,1460];w,h=crop[2]-crop[0],crop[3]-crop[1]
elevation=math.radians(35);center=Vector(((crop[0]+crop[2])/2,-(crop[1]+crop[3])/2/math.sin(elevation),0))
backward=Vector((0,-math.cos(elevation),math.sin(elevation)))
data=bpy.data.cameras.new('York pair texture native context');camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera)
data.type='ORTHO';data.ortho_scale=max(w,h);data.clip_start=.01;data.clip_end=20000
camera.location=center+backward*10000;camera.rotation_euler=(-backward).to_track_quat('-Z','Y').to_euler()
scene.render.resolution_x=w*4;scene.render.resolution_y=h*4
render_views(scene.name,{'native':camera.name},OUT/'native-joint',width=w*4,modes=('textured',))
print(OUT)
