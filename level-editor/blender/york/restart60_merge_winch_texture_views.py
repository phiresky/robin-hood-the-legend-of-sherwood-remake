"""Union separately projected underside samples without replacing first-pass/source pixels."""
import hashlib,json,shutil,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/active/york/restart2';targeted='--targeted' in sys.argv;A=W/('restart60-winch-chain-combined-v1' if targeted else 'restart60-winch-chain-texture-bake-v1');B=W/('restart60-winch-chain-targeted2-bake-v1' if targeted else 'restart60-winch-chain-underside-bake-v1');O=W/('restart60-winch-chain-combined-v2' if targeted else 'restart60-winch-chain-combined-v1');assert not O.exists()
sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest();assert shutil.disk_usage(ROOT).free>10*1024**3;assert int(next(l.split()[1] for l in Path('/proc/meminfo').read_text().splitlines() if l.startswith('MemAvailable:')))*1024>6*1024**3
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire
acquire(slots=2)
import bpy,numpy as np
from refinement_workspace import _geometry
from workspace_components import appearance_state
rb=json.loads((B/'validation.json').read_text());ra=json.loads((A/'validation.json').read_text());names={r['object'] for r in ra['layers'][0]['objects']};assert names=={r['object'] for r in rb['layers'][0]['objects']}
def image_for(o):
 mats={o.data.materials[p.material_index] for p in o.data.polygons};assert len(mats)==1;mat=next(iter(mats));return next(n.image for n in mat.node_tree.nodes if n.type=='TEX_IMAGE' and n.image)
def pixels(im):
 a=np.empty(len(im.pixels),np.float32);im.pixels.foreach_get(a);return a.reshape(im.size[1],im.size[0],4)
bpy.ops.wm.open_mainfile(filepath=str(B/'worker.blend'));samples={}
for row in rb['layers'][0]['objects']:
 o=bpy.context.scene.objects[row['object']];samples[o.name]=(pixels(image_for(o)),np.load(row['texel_provenance']['path'])['ownership'],row['texel_provenance']['uv_sha256'])
bpy.ops.wm.open_mainfile(filepath=str(A/'worker.blend'));scene=bpy.context.scene;outside={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.name not in names};guards=[]
for frame in range(45):
 scene.frame_set(frame*2);bpy.context.view_layer.update();guards.append({o.name:_geometry(o) for o in scene.objects})
scene.frame_set(88);bpy.context.view_layer.update();O.mkdir();(O/'provenance-0').mkdir();rows=[]
for row in ra['layers'][0]['objects']:
 o=scene.objects[row['object']];im=image_for(o);before=pixels(im);owner=np.load(row['texel_provenance']['path'])['ownership'];sample,second,uvhash=samples[o.name];assert uvhash==row['texel_provenance']['uv_sha256'];assert before.shape==sample.shape
 replace=(owner==0)&(second==2);after=before.copy();after[replace]=sample[replace];assert np.array_equal(before[~replace],after[~replace]);im.pixels.foreach_set(after.ravel());im.update();im.pack();owner[replace]=2
 dest=O/'provenance-0'/Path(row['texel_provenance']['path']).name;np.savez_compressed(dest,ownership=owner);row['texel_provenance'].update(path=str(dest),sha256=sha(dest),packed_image_sha256=hashlib.sha256(im.packed_file.data).hexdigest(),rgba8_sha256=hashlib.sha256(np.rint(np.clip(after,0,1)*255).astype(np.uint8).tobytes()).hexdigest());rows.append({'object':o.name,'added_underside_samples_including_padding':int(replace.sum()),'prior_source_and_generated_exact':True})
assert outside=={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.name in outside}
for frame in range(45):
 scene.frame_set(frame*2);bpy.context.view_layer.update();assert guards[frame]=={o.name:_geometry(o) for o in scene.objects}
scene.frame_set(88);bpy.context.view_layer.update();bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(O/'worker.blend'),compress=True)

if 'counts' in ra:ra['first_pass_counts']=ra.pop('counts')
if 'supplementary_merge' in ra:ra.setdefault('supplementary_merge_history',[]).append(ra.pop('supplementary_merge'))
ra['combined_counts_note']='Use surface-coverage.json for combined on-surface coverage; first_pass_counts are historical and not combined totals.'
ra['supplementary_merge']={'source_models':[{'path':str(p/'worker.blend'),'sha256':sha(p/'worker.blend')} for p in [A,B]],'method':'Copy second-pass generated samples only where first-pass provenance is0. All original source and first-pass generated samples retained byte-identically. No texture extrapolation.','objects':rows};(O/'validation.json').write_text(json.dumps(ra,indent=2)+'\n');(O/'all-pose-preservation.json').write_text(json.dumps({'status':'PASS','poses':45,'geometry_exact':True,'outside_appearance_exact':True,'protected_objects':len(outside),'receiver_count':85,'saved_model_sha256':sha(O/'worker.blend'),'added_underside_samples_including_padding':sum(r['added_underside_samples_including_padding'] for r in rows)},indent=2)+'\n');print(O)
