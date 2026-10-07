"""Bind well native colors, separate pail inference and saved geometry evidence."""
import hashlib,json
from pathlib import Path
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
B=ROOT/'level-editor/work/york-refinement';D=B/'restart7-market-well-v3'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def main():
 construction=json.loads((D/'construction.json').read_text());assert sha(D/'model.blend')==construction['model_sha256']
 box=construction['native_box'];rgb=np.array(Image.open(B/'baseline/covered.png').convert('RGBA').crop(box))[:,:,:3];domain=np.array(Image.open(D/'native-domain.png'));pail=np.array(Image.open(D/'bucket-inferred-domain.png'));native=np.zeros(domain.shape,dtype=np.uint8);level=json.loads((B/'baseline/york.rhp.json').read_text())
 for i in [49,50]:
  m=level['masks'][i];a=np.array(Image.open(B/f'baseline/masks/{i:06d}.png').convert('L'));x,y=m['box_top_left'];x-=box[0];y-=box[1];native[y:y+a.shape[0],x:x+a.shape[1]]=np.maximum(native[y:y+a.shape[0],x:x+a.shape[1]],a)
 assert np.array_equal(domain,np.maximum(native,pail));ownership=np.zeros(domain.shape,dtype=int);rows=[]
 for name,r in construction['ownership'].items():
  p=Path(r['atlas']);assert sha(p)==r['sha256'];a=np.array(Image.open(p));known=a[:,:,3]>0;assert np.array_equal(a[:,:,:3],rgb);assert np.array_equal(a[:,:,3][known],domain[known]);ownership+=known;rows.append(dict(object=name,atlas_sha256=sha(p),accepted_pixels=int(known.sum()),rgb_exact=True,accepted_domain_alpha_exact=True))
 assert ownership.max()==1
 report=dict(status='PASS native RGB and per-receiver ownership; geometry review is separate',model_sha256=construction['model_sha256'],native_mask_union_pixels=int((native>0).sum()),pail_additional_traced_pixels=int(((pail>0)&(native==0)).sum()),all_domain_pixels=int((domain>0).sum()),accepted_unique_pixels=int((ownership>0).sum()),native_accepted_pixels=int(((ownership>0)&(native>0)).sum()),pail_domain_accepted_pixels=int(((ownership>0)&(pail>0)).sum()),duplicate_accepted_pixels=int((ownership>1).sum()),atlases=rows,context_geometry_uv_matrix_guard=construction['context_exact'],source_sha256=construction['source_sha256'],authority_sha256=sha(B/'restart7-market-well-study-v1/authority.json'),inference='Small pail source domain follows a separately traced bright rim and compact dark body. It is not a native gameplay mask; broad surrounding shadow is excluded.',limitations=['Accepted original pixels are retained exactly; physical source coverage and outside-domain projections remain separately audited.','Unobserved masonry, timber, roof underside and pail backs remain neutral gray.','Grazing roof/source projection can stretch low-resolution observed colors from oblique views; no synthesized appearance has been approved.'],publication=False,user_geometry_approved=False,texture_approved=False)
 (D/'source-guard.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:v for k,v in report.items()if k not in ['atlases','context_geometry_uv_matrix_guard']}))
if __name__=='__main__':main()
