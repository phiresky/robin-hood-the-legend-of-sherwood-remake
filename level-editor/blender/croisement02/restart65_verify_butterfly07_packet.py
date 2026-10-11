"""Check frozen source timing, fixed anatomy and exact observation RGB provenance."""
import json
from pathlib import Path
import numpy as np
from PIL import Image
from restart64_butterfly07_source_ownership import ROOT,prepare,sha,metrics,WING
BASE=ROOT/'level-editor/work/croisement02-refinement'
FIT=BASE/'restart64-butterfly07-pose-refinement-v1/fit.json';ATLAS=BASE/'restart65-butterfly07-exact-observation-atlas-v1'
def main():
 fit=json.loads(FIT.read_text());parent=json.loads((BASE/'restart14-butterflies/butterfly07-v3-baseline-contacts-v1/joined-fit.json').read_text());assert fit['fixed_geometry']==parent['fixed_geometry'];assert len(fit['rows'])==99
 rows=[prepare(row) for row in fit['rows']]
 for i,(a,b) in enumerate(zip(fit['rows'],parent['rows'])):
  assert a['source']==b['source'] and a['fixed_path_anchor_zup']==b['fixed_path_anchor_zup'];assert a['source']['first_tick']==2*i and a['source']['duration_ticks']==2
  assert np.max(np.abs(np.array(a['parameters'][:5])-b['parameters'][:5]))<=8+1e-8;assert np.max(np.abs(np.array(a['parameters'][5:])-b['parameters'][5:]))<=.4+1e-8
 counts=metrics(WING,rows);assert counts==dict(missing=250,bright_missing=54,extra=578,source=5015)
 verified=0
 for domain in range(4):
  image=np.array(Image.open(ATLAS/f'wing-{domain}.png'));known=np.array(Image.open(ATLAS/f'wing-{domain}-known.png'))>0;phases=np.load(ATLAS/f'wing-{domain}-source-phase.npy');h,w=known.shape
  for phase in np.unique(phases[known]):
   yy,xx=np.nonzero(known&(phases==phase));uv=np.c_[(xx+.5)/w*8,(yy+.5)/h*13-7];c=rows[int(phase)];m=c['maps'][domain//2];assert m['face']==domain%2
   xy=uv@m['matrix'][:2,:2].T+m['offset'][:2]+c['shift'];ix=np.floor(xy[:,0]).astype(int);iy=np.floor(xy[:,1]).astype(int)
   assert np.array_equal(image[yy,xx,:3],c['rgba'][iy,ix,:3]);assert np.all(c['rgba'][iy,ix,3]>0);verified+=len(xx)
 result=dict(status='PASS_SOURCE_CLOCK_GEOMETRY_AND_EXACT_RGB_PROVENANCE',fit_sha256=sha(FIT),atlas_report_sha256=sha(ATLAS/'report.json'),phases=99,cycle_ticks=198,known_wing_texels_verified=verified,source_geometry_metrics=counts,limits=['Provenance does not assert complete source silhouette or appearance agreement.','Contact audit and saved-model renderer are separate checks.'])
 out=BASE/'restart65-butterfly07-private-gallery-v1/verification.json';assert not out.exists();out.write_text(json.dumps(result,indent=2)+'\n');print(result)
if __name__=='__main__':main()
