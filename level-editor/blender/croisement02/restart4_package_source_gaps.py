"""Package reviewed bounded stump and shed corrections without replacing saved materials."""
import sys,json,shutil,hashlib
from pathlib import Path
import bpy,numpy as np
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json,record_recipe
from render_slots import acquire,release
from refinement_workspace import prepare,modified,validate

SPECS={
 'stump':('stump-cap-v5','croisement02-logging-clearing-stumps','scenery-round-1/assets/croisement02-logging-clearing-stumps','texture-fill-round-1/croisement02-logging-clearing-stumps/experiment/bake-v1/worker.blend'),
 'shed':('shed-east-v2','croisement02-woodcutters-shed','restart2-vegetation/shed-package-v1/assets/croisement02-woodcutters-shed','restart2-textures/approved-prop-repairs-fill-v1/croisement02-woodcutters-shed/stored-preparation-v3/experiment/native-front-retained-v1/worker.blend')}

def saved_audit(w,guard):
 rows=[]
 for o in bpy.data.collections['Croisement02 Working'].all_objects:
  if o.type!='MESH' or o.get('asset_group')!=w.name:continue
  assert all(np.isfinite(v.co).all() for v in o.data.vertices)
  mats=[]
  for i in sorted({p.material_index for p in o.data.polygons}):
   m=o.data.materials[i];images=[n.image for n in m.node_tree.nodes if n.type=='TEX_IMAGE' and n.image]
   if images:
    assert all(im.packed_file for im in images)
    mats.append(dict(name=m.name,images=[dict(name=im.name,size=list(im.size),packed_sha256=hashlib.sha256(im.packed_file.data).hexdigest()) for im in images]))
   else:
    outputs=[n for n in m.node_tree.nodes if n.type=='OUTPUT_MATERIAL'];assert len(outputs)==1
    node=outputs[0].inputs['Surface'].links[0].from_node
    assert node.type=='EMISSION' and not any(s.is_linked for s in node.inputs)
    mats.append(dict(name=m.name,images=[],source_role='Explicitly inferred neutral hidden surface',emission_color=list(node.inputs['Color'].default_value)))
  rows.append(dict(object=o.name,source_node=o.get('source_node'),vertices=len(o.data.vertices),faces=len(o.data.polygons),used_materials=mats))
 assert rows
 write_json(w/'inspection/saved-model-audit.json',dict(status='PASS',model_sha256=sha(w/'model.blend'),objects=rows,saved_payload_guard=guard,ownership_note='Native overlay alpha is bounded by the reviewed source mask; original atlas provenance remains independent.'))

def main(key):
 label,asset,previous,parent=SPECS[key];previous=OUT/previous;parent=OUT/parent;trial=OUT/'restart4-source-gaps'/label;out=OUT/'restart4-source-gaps/packaged-v1/assets'/asset
 proof=json.loads((trial/'root-review.json').read_text());h=sha(trial/'model.blend');assert proof['model_sha256']==h and proof['status'].startswith('PASS scoped')
 guard=json.loads((trial/'saved-guard.json').read_text());assert guard['status']=='PASS' and guard['model_sha256']==h and guard['parent_model_sha256']==sha(parent)
 cfg=json.loads((previous/'workspace.json').read_text());report=json.loads((previous/'inspection/refinement.json').read_text())
 acquire()
 try:
  if not out.exists():
   bpy.ops.wm.open_mainfile(filepath=str(parent));bpy.context.preferences.filepaths.save_version=0
   prepare(out,asset_id=asset,scene_name=cfg['scene_name'],collection_name=cfg['collection_name'],source_path=previous/'reference/source.png',grouping_manifest=previous/'reference/grouping.json',inventory_path=previous/'reference/inventory.json',review_path=previous/'reference/grouping-review.json',source_mask_manifest=previous/'source-masks.json',width=384,height=384,framing_padding=1.3,lighting=cfg['lighting'])
  elif not (out/'input/views.json').exists():raise ValueError('Incomplete prepare needs explicit reconciliation')
  inspection=out/'inspection';inspection.mkdir(exist_ok=True)
  shutil.copyfile(trial/'model.blend',out/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(out/'model.blend'))
  report.update(model_sha256=h,status='Source-gap correction; pending user geometry approval',limitations=proof['notes'])
  write_json(inspection/'refinement.json',report)
  if not (out/'modified/views.json').exists():modified(out)
  shutil.copyfile(trial/'model.blend',out/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(out/'model.blend'))
  write_json(out/'validation.json',validate(out));saved_audit(out,guard)
  for name in ['root-review.json','proposal.json','saved-guard.json','source-comparison.png']:
   shutil.copyfile(trial/name,inspection/name)
  shutil.copytree(trial/'wide-contact-v1',inspection/'wide-contact',dirs_exist_ok=True)
  record_recipe(out,__file__)
  write_json(inspection/'prototype-preservation.json',dict(model_sha256=h,parent_model=str(parent),parent_model_sha256=sha(parent),previous_workspace=str(previous),candidate_worker=str(trial),candidate_model_sha256=h,approval='pending',prior_texture_approval_inherited=False))
  assert sha(out/'model.blend')==h
 finally:release()
 print(out)
if __name__=='__main__':main(sys.argv[sys.argv.index('--')+1])
