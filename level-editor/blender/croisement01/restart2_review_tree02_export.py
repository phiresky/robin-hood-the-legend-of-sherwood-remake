"""Reopen exported approved GLB and render it through the approved eight cameras."""
import json
import shutil
import sys
from pathlib import Path
import bpy
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
sys.path.insert(0,str(Path(__file__).parent))
from render_slots import acquire
from review_evidence import sha
import render_candidate
R=ROOT/'level-editor/work/active/croisement01/restart2'
identifier='croisement01-tree-02'
case=R/'approved-tree02-fill-v1'/identifier
stage=R/'tree02-approved-integration-v1';asset=stage/'assets'/identifier
experiment='experiment'
assert shutil.disk_usage(R).free >= 10*1024**3+128*1024**2
assert next(int(line.split()[1]) for line in Path('/proc/meminfo').read_text().splitlines() if line.startswith('MemAvailable:')) >= 6*1024**2
acquire(slots=2)
@bpy.app.handlers.persistent
def enforce_review_threads(scene, *_):
    scene.render.threads_mode='FIXED';scene.render.threads=2
bpy.app.handlers.render_pre.append(enforce_review_threads)
bpy.ops.wm.open_mainfile(filepath=str(R/'ready-tree02-texture-v3/model.blend'))
config=json.loads((R/'tree02-v8/assets'/identifier/'workspace.json').read_text())
scene=bpy.data.scenes[config['scene_name']];bpy.context.window.scene=scene
collection=bpy.data.collections[config['collection_name']]
own=[o for o in collection.all_objects if o.type=='MESH' and o.get('asset_group')==config['asset_id']]
def bounds(objects):
    points=[o.matrix_world@v.co for o in objects for v in o.data.vertices]
    return [[min(p[i] for p in points) for i in range(3)],[max(p[i] for p in points) for i in range(3)]]
reviewed_own=own
expected=bounds(reviewed_own)
for obj in own: bpy.data.objects.remove(obj,do_unlink=True)
before=set(bpy.data.objects)
bpy.ops.import_scene.gltf(filepath=str(asset/'model.glb'))
added=set(bpy.data.objects)-before
pivot=Vector(json.loads((asset/'asset.json').read_text())['source_origin_scene'])
for obj in added:
    if obj.parent not in added: obj.location+=pivot
bpy.context.view_layer.update()
meshes=[o for o in added if o.type=='MESH']
actual=bounds(meshes)
error=max(abs(a-b) for row1,row2 in zip(expected,actual) for a,b in zip(row1,row2))
if error>.002:raise ValueError('GLB world bounds differ: '+str(dict(expected=expected,actual=actual,error=error)))
for obj in meshes:
    for coll in list(obj.users_collection): coll.objects.unlink(obj)
    collection.objects.link(obj);obj['asset_group']=config['asset_id']
packet=json.loads((case/experiment/'views.json').read_text());packet['object_names']=[o.name for o in meshes];packet.pop('render_object_names',None)
out=stage/'glb-review-v1';out.mkdir(exist_ok=False);(out/'inspection').mkdir();(out/'modified').mkdir()
(out/'workspace.json').write_text(json.dumps(config,indent=2)+'\n')
(out/'modified/views.json').write_text(json.dumps(packet,indent=2)+'\n')
# The GLB appearance review needs only this asset, not another whole-map save.
for other in list(bpy.data.objects):
    if other.type=='MESH' and other not in meshes:bpy.data.objects.remove(other,do_unlink=True)
bpy.data.orphans_purge(do_recursive=True)
bpy.context.preferences.filepaths.save_version=0
bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'),compress=True)
(out/'glb-proof.json').write_text(json.dumps(dict(source_glb_sha256=sha(asset/'model.glb'),expected_bounds=expected,actual_bounds=actual,max_bound_error=error,mesh_count=len(meshes)),indent=2)+'\n')
sys.argv=['render','--',str(out)];render_candidate.main()
