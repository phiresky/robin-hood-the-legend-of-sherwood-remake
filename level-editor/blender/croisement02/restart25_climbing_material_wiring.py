"""Connect inferred climbing texture RGB without touching observed materials."""
import sys,json,hashlib,shutil
from pathlib import Path
import bpy,numpy as np
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from render_slots import acquire,release
from restart24_bake_climbing_donors import retained
from bake_texture_candidate import snapshot
BASE=OUT/'restart14-hidden-archer';DEST=BASE/'climbing-texture-bake-v3'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def main(state):
 assert shutil.disk_usage(BASE).free>=10*1024**3
 assert int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024>=6*1024**3
 src=BASE/f'climbing-texture-bake-v2/profile-05-{state}'
 record=json.loads((src/'bake-validation.json').read_text());model=src/'model.blend';assert sha(model)==record['model_sha256']
 dest=DEST/f'profile-05-{state}';assert not dest.exists();dest.mkdir(parents=True)
 bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;obj=next(o for o in scene.objects if o.type=='MESH');before=retained(obj)
 known=lambda:{k:v for k,v in snapshot(scene,{obj.name})['physical_foliage'].items() if v['known_rgba'] is not None}
 observed=known();changes=[]
 for slot,mat in enumerate(list(obj.data.materials)):
  if not mat or mat.get('foliage_observed'):continue
  faces=[f for f in obj.data.polygons if f.material_index==slot]
  own=obj.data.color_attributes['Source ownership']
  if not faces or any(own.data[i].color[0]!=0 for f in faces for i in f.loop_indices):continue
  tex=next(n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE')
  shader=next(n for n in mat.node_tree.nodes if n.type=='BSDF_PRINCIPLED')
  missing=[name for name in ['Base Color','Emission Color'] if not shader.inputs[name].links]
  if not missing:continue
  assert mat.get('generated_source_sha256'),'Only generated inferred material may be connected'
  replacement=mat.copy();tex=replacement.node_tree.nodes[tex.name];shader=replacement.node_tree.nodes[shader.name]
  for name in missing:replacement.node_tree.links.new(tex.outputs['Color'],shader.inputs[name])
  replacement['texture_provenance']='Generated inferred reverse material; residual unfilled RGB retains own paired front leaf donor, never new observed evidence.'
  obj.data.materials[slot]=replacement
  changes.append(dict(slot=slot,material=mat.name,connected=missing,physical_alpha_unchanged=True,backfacing_unchanged=True,packed_texture_unchanged=True))
 assert len(changes)==1 and retained(obj)==before and known()==observed
 bpy.context.preferences.filepaths.save_version=0;scene.render.threads_mode='FIXED';scene.render.threads=2
 bpy.ops.wm.save_as_mainfile(filepath=str(dest/'model.blend'),compress=True)
 bpy.ops.wm.open_mainfile(filepath=str(dest/'model.blend'));scene=bpy.context.scene;obj=next(o for o in scene.objects if o.type=='MESH')
 assert retained(obj)==before and known()==observed
 construction=json.loads((src/'construction.json').read_text());construction['model_sha256']=sha(dest/'model.blend')
 (dest/'construction.json').write_text(json.dumps(construction,indent=2)+'\n')
 record.update(status='SAVED_REOPEN_PRESERVATION_PASS; appearance/native guard pending',parent_model_sha256=sha(model),model_sha256=construction['model_sha256'],inferred_shader_wiring=changes,recipe_sha256=sha(__file__))
 (dest/'bake-validation.json').write_text(json.dumps(record,indent=2)+'\n')
 assert sum(p.stat().st_size for p in DEST.rglob('*') if p.is_file())<32*1024**2
 print(json.dumps(dict(state=state,model_sha256=record['model_sha256'],changes=changes)),flush=True)
if __name__=='__main__':
 acquire(slots=2)
 try:main(sys.argv[sys.argv.index('--')+1])
 finally:release()
