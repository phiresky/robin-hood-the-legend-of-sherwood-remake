"""Bind additive root receivers and immutable native/previously filled surfaces."""
import hashlib,json
from pathlib import Path
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/croisement02-refinement'
def sha(p):
 with Path(p).open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
def main():
 dest=OUT/'restart3-root-texture-constraints-v1';dest.mkdir(exist_ok=False)
 records=[]
 for kind,asset,new_name,trial,old_trial,masks in [
 ('logging','logging-clearing-log','Logging root and branch tangle','logging-convex-v7','logging-branches-v5',[108]),
 ('southwest','southwest-stumps','Southwest stump root and branch tangle','southwest-convex-v3','southwest-branches-v1',[105,106])]:
  asset='croisement02-'+asset;worker=OUT/f'restart2-vegetation/{kind}-root-package-v1/assets'/asset
  audit_path=worker/'inspection/saved-model-audit.json';audit=json.loads(audit_path.read_text());model=sha(worker/'model.blend');assert audit['model_sha256']==model
  added=next(o for o in audit['objects'] if o['object']==new_name);others=[o for o in audit['objects'] if o['object']!=new_name]
  source=OUT/'restart2-vegetation'/old_trial/'front-source.png';image=np.asarray(Image.open(source).convert('RGBA'));native_entry=next(r for r in added['used_materials'][0]['images'] if r['name']=='front-source.png');assert sha(source)==native_entry['packed_sha256']
  domain=dest/(kind+'-observed-domain.png');Image.fromarray(image[:,:,3]).save(domain)
  guard_path=OUT/'restart2-vegetation'/trial/'evidence.json';guard=json.loads(guard_path.read_text());assert guard['model_sha256']==model
  protected={o['object']:guard['protected_appearance'][o['object']] for o in others}
  cfg=json.loads((worker/'source-masks.json').read_text());assignment=next(r for r in cfg['projections']['exterior']['assignments'] if r.get('asset_group')==asset)
  records.append(dict(asset_id=asset,worker=str(worker),model_sha256=model,editable_object=new_name,
   original_meshes_fully_protected=protected,original_materials_and_images=[dict(object=o['object'],materials=o['used_materials'])for o in others],
   native_material=added['used_materials'][0]['name'],native_material_index=0,native_image=str(source),native_image_sha256=sha(source),native_image_policy='Keep entire packed front-source.png exact; its alpha chooses observed source RGB in front material. Do not treat inferred root shape as permission to edit these native colors.',
   protected_domain=str(domain),protected_domain_sha256=sha(domain),native_domain_pixels=int((image[:,:,3]>0).sum()),native_masks=masks,excluded_masks=assignment.get('exclude_mask_indices',[]),
   native_uv_map='Native front projection',native_uv_formula=['worldX/1792','1-(-worldY*sin(35deg)-worldZ*cos(35deg))/1152'],
   editable_appearance=['Material0 own-native-board-supplement fallback only where front-source alpha is zero','Material1 Inferred branch bark, all its rear faces'],
   inferred_uv_map='Inferred board grain',inferred_material_index=1,inferred_material=added['used_materials'][1]['name'],
   framing='Use complete saved-material eight views; legacy frozen source-only frames crop new root extensions.',
   audit=str(audit_path),audit_sha256=sha(audit_path),preservation_evidence=str(guard_path),preservation_evidence_sha256=sha(guard_path),source_mask_manifest=str(worker/'source-masks.json'),source_mask_manifest_sha256=sha(worker/'source-masks.json')))
 (dest/'receivers.json').write_text(json.dumps(dict(status='Exact approved receiver constraints; no model or source ownership changes',records=records),indent=2)+'\n');print(dest)
if __name__=='__main__':main()
