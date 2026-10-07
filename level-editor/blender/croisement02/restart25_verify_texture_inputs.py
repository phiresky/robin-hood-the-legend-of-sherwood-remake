"""Verify exact protected pixels and freeze own-source supplemental references."""
import hashlib,json,shutil,sys
from pathlib import Path
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'level-editor/work/croisement02-refinement'
OUT=BASE/'restart25-approved-state-materialization-v1'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def main(relative):
 worker=OUT/relative;d=json.loads((worker/'derivation.json').read_text());assert sha(d['source_model'])==d['source_model_sha256'];assert sha(worker/'model.blend')==d['prepared_model_sha256'];assert d['geometry_uv_materials_unchanged']
 results=[]
 for v in d['views']:
  i=v['view'];stored=np.array(Image.open(worker/'stored'/f'view-{i}-textured.png').convert('RGBA'));guide=np.array(Image.open(worker/'modified/views'/f'view-{i}-textured.png').convert('RGBA'));known=np.array(Image.open(worker/'modified/views'/f'view-{i}-known.png').convert('RGBA'))[:,:,0]>=128
  assert np.array_equal(stored[known],guide[known]);assert np.array_equal(stored[:,:,3],guide[:,:,3]);assert np.all(guide[~known,:3]==77);assert int((~known).sum())==v['editable_pixels']
  results.append({'view':i,'protected_pixels':int(known.sum()),'editable_pixels':v['editable_pixels'],'protected_rgba_exact':True,'full_alpha_exact':True})
 assert results[0]['editable_pixels']==0,'Native source-facing view must be untouched'
 target=worker/'source-reference-review';target.mkdir(exist_ok=False)
 refs=[]
 for tag,path in [('hole-initial',OUT/'hole-texture-inputs-v1/initial/derivation.json'),('hole-applied',OUT/'hole-texture-inputs-v1/applied/derivation.json'),('mound-initial',BASE/'restart9-hiding-scatter/mound-flat-v2/validation.json')]:
  meta=json.loads(path.read_text());source=Path(meta.get('source_image',meta.get('source')));expected=meta.get('source_sha256');assert sha(source)==expected
  dest=target/(tag+'.png');shutil.copyfile(source,dest)
  refs.append({'source':'material','file':str(dest),'sha256':sha(dest),'parent':str(source),'parent_sha256':expected,'role':'Own native leaf litter color and material detail only. Keep the requested target geometry, silhouette, layout and every protected pixel; do not transplant this reference shape.'})
 auxiliary={'input_sha256':sha(worker/'private-inputs/input.png'),'lighting_sha256':sha(worker/'private-inputs/solid.png'),'references':refs}
 (target/'auxiliary-references.json').write_text(json.dumps(auxiliary,indent=2)+'\n')
 evidence={str(p.relative_to(worker)):sha(p) for p in worker.rglob('*') if p.is_file() and p.suffix in ['.png','.json']}
 report={'status':'PASS_PROTECTED_INPUT_PIXELS_ROOT_VISUAL_REVIEW_PENDING','source_model_sha256':d['source_model_sha256'],'prepared_model_sha256':d['prepared_model_sha256'],'derivation_sha256':sha(worker/'derivation.json'),'views':results,'artifacts':evidence,'references':str(target/'auxiliary-references.json'),'generation_started':False,'final_appearance_approved':False}
 (target/'verification.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'worker':relative,'known_exact':True,'alpha_exact':True,'editable_pixels':sum(v['editable_pixels'] for v in results),'verification_sha256':sha(target/'verification.json')}))
if __name__=='__main__':main(sys.argv[1])
