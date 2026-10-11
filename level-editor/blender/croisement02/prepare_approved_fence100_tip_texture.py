"""Prepare source-limited texture inputs for the explicitly approved tiny rail tip."""
import sys,json,hashlib,math,shutil
from pathlib import Path
import bpy
R=Path.cwd();sys.path[:0]=[str(R/'level-editor/refinement'),str(R/'level-editor/refinement/blender')]
from render_slots import acquire,release
from refinement_review import render_review
from PIL import Image
B=R/'level-editor/work/croisement02-refinement';A=B/'restart3-review-batches/six-scoped-reviews-oct11-v1/user-approval.json';D=B/'approved-fence100-tip-texture-oct11-v2';BASE=B/'static-terrain-closure-oct11-v1/fence100-tip-candidate-v2';ASSET='croisement02-southwest-field-wattle-fence'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 assert sha(A)=='d435a76b08eb3ac616f1ae2ffcdf19a7875969e515dd01f65de1871ca886ef1d'
 decision=next(r for r in json.loads(A.read_text())['decisions']if r['asset_id']==ASSET+'--rail-tip');model=Path(decision['model']);assert sha(model)==decision['model_sha256']
 acquire(slots=2)
 try:
  assert shutil.disk_usage(R).free>=10*1024**3;assert int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024>=6*1024**3;D.mkdir(exist_ok=False)
  im=Image.new('L',(1792,1152));points=json.loads((BASE/'candidate.json').read_text())['native_centers']
  for x,y in points:im.putpixel((x,y),255)
  im.save(D/'known-tip-six.png');inventory=dict(masks=[dict(index=10100,png='known-tip-six.png',box_top_left=[0,0],box_size=[1792,1152])]);(D/'mask-inventory.json').write_text(json.dumps(inventory,indent=2)+'\n');manifest=dict(version=1,mask_inventory=str(D/'mask-inventory.json'),projections={'exterior':{'source_sha256':sha(B/'source-states/covered.png'),'state':'Approved static elevated tip six native centers only','assignments':[dict(source_node='building-023',projection_component='source100-tip-candidate',mask_indices=[10100],reviewed=True)]}});(D/'source-masks.json').write_text(json.dumps(manifest,indent=2)+'\n')
  bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;scene.render.threads_mode='FIXED';scene.render.threads=2
  frames=json.loads((BASE/'close-review/views.json').read_text());frames.pop('render_object_names',None)
  render_review(D/'packet',scene_name=scene.name,collection_name='Croisement02 Working',asset_id=ASSET,source_path=B/'source-states/covered.png',frame_manifest=frames,source_mask_manifest=D/'source-masks.json',render_object_names=['Source100 elevated wicker tip'],allow_projection_revision=True,allow_mask_revision=True)
  (D/'derivation.json').write_text(json.dumps(dict(status='SOURCE_ONLY_DERIVATIVE_PENDING_AGENT_REVIEW',original_geometry_decision=decision,approval_receipt=str(A),approval_sha256=sha(A),native_known_centers=points,scope='Approved tip only. Six source-supported pixels protected; retained fence contributes physical occlusion context and supplementary appearance, no editable original fence surfaces.',files={str(p):sha(p)for p in (D/'packet').rglob('*')if p.is_file()}),indent=2)+'\n')
 finally:release()
if __name__=='__main__':main()
