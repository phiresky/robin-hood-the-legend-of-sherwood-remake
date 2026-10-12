"""Bake reviewed chain fill privately, guarding all poses and stable component appearance."""
import hashlib,json,shutil,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/active/york/restart2';suffix=sys.argv[sys.argv.index('--')+1] if '--' in sys.argv else 'texture';I=W/('restart60-winch-chain-'+suffix+'-inputs-v1');E=I/'experiment';O=W/('restart60-winch-chain-'+suffix+'-bake-v1');sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
assert not O.exists();assert shutil.disk_usage(ROOT).free>10*1024**3
assert int(next(l.split()[1] for l in Path('/proc/meminfo').read_text().splitlines() if l.startswith('MemAvailable:')))*1024>6*1024**3
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire
acquire(slots=2)
import bpy
from refinement_workspace import _geometry
from bake_reviewed_asset import stage
bpy.ops.wm.open_mainfile(filepath=str(I/'model.blend'));scene=bpy.context.scene;scene.render.threads_mode='FIXED';scene.render.threads=2;bpy.context.preferences.filepaths.save_version=0
names=set(json.loads((I/'input-review.json').read_text())['object_names']);guards=[];outside=[]
for frame in range(45):
 scene.frame_set(frame*2);bpy.context.view_layer.update();guards.append({o.name:_geometry(o) for o in scene.objects});outside.append({o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.name not in names})
scene.frame_set(88);bpy.context.view_layer.update()
g=E/'generation-short-no-mask-with-lighting-openrouter-with-auxiliary/generated-preserved.png'
r=stage(E/'views.json',g,O,texels_per_unit=2)
for frame in range(45):
 scene.frame_set(frame*2);bpy.context.view_layer.update();assert guards[frame]=={o.name:_geometry(o) for o in scene.objects};assert outside[frame]=={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.name not in names}
scene.frame_set(88);bpy.context.view_layer.update()
(O/'all-pose-preservation.json').write_text(json.dumps({'status':'PASS','poses':45,'geometry_exact':True,'outside_appearance_exact':True,'protected_objects':len(outside[0]),'receiver_count':len(names),'saved_model_sha256':sha(O/'worker.blend'),'source_model_sha256':sha(I/'model.blend'),'generated_preserved_sha256':sha(g)},indent=2)+'\n');print(O,flush=True)
