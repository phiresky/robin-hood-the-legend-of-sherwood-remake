"""Normalize private Tree07 labels without changing the reviewed surface."""
import sys,hashlib,json,shutil
from pathlib import Path
import bpy,numpy as np
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from evidence_io import sha,write_json
B=ROOT/'level-editor/work/croisement03-refinement/restart2'
def surface(scene):
 rows={}
 for o in scene.objects:
  if o.type!='MESH':continue
  m=o.data
  rows[o.name]=dict(vertices=[list(v.co) for v in m.vertices],faces=[list(f.vertices) for f in m.polygons],uvs={u.name:[list(v.uv) for v in u.data] for u in m.uv_layers},matrix=[list(v) for v in o.matrix_world],materials=[m.name for m in m.materials])
 return rows
def main():
 src=B/'tree07-crown-prototype-v2';out=B/'tree07-crown-prototype-v3';out.mkdir(exist_ok=False);acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(src/'worker.blend'));scene=bpy.data.scenes['Tree13 isolated wood'];before=surface(scene);images={i.name:hashlib.sha256(np.asarray(i.pixels[:],np.float32).tobytes()).hexdigest() for i in bpy.data.images if i.has_data};changes=[]
  for o in scene.objects:
   if o.type!='MESH' or o.get('asset_group')=='croisement03-arbre07-fragment-tree07-provisional':continue
   changes.append(dict(name=o.name,old=o.get('asset_group'),new='croisement03-tree-07'));o['asset_group']='croisement03-tree-07'
  assert surface(scene)==before
  assert images=={i.name:hashlib.sha256(np.asarray(i.pixels[:],np.float32).tobytes()).hexdigest() for i in bpy.data.images if i.has_data}
  bpy.data.libraries.write(str(out/'worker.blend'),{scene},fake_user=True,compress=True)
  shutil.copytree(src/'actual',out/'actual');d=json.loads((src/'receipt.json').read_text());d.update(model_sha256=sha(out/'worker.blend'),source_model_sha256=sha(src/'worker.blend'),metadata_normalization=changes);write_json(out/'receipt.json',d)
  write_json(out/'metadata-normalization.json',dict(status='PASS metadata-only normalized asset groups',source_model_sha256=sha(src/'worker.blend'),model_sha256=sha(out/'worker.blend'),geometry_uv_transform_material_rgba_exact=True,surface_sha256=hashlib.sha256(json.dumps(before,sort_keys=True).encode()).hexdigest(),images=images,changes=changes,actual_evidence_inherited={str(p.relative_to(out)):sha(p) for p in (out/'actual').glob('*') if p.is_file()}))
 finally:release()
if __name__=='__main__':main()
