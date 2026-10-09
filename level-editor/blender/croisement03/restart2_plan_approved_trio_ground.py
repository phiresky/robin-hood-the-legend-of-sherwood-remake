"""Audit reserved atlas domains and write the concrete private integration handoff."""
import hashlib,json
from pathlib import Path
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];B=ROOT/'level-editor/work/croisement03-refinement/restart2';O=B/'approved-trio-ground-integration-v1'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def read(p):return json.loads(Path(p).read_text())
def main():
 report=read(O/'report.json');assert sha(O/'model.glb')==report['model_sha256'];assert all(sha(p)==h for p,h in report['pins'].items());e=B/'approved-hub-textures-v1/trio-ground/correction-v3-region-guide/experiment';mask=np.array(Image.open(e/'mask.png').convert('RGBA'))[:,:,3]==0;manifest=read(B.parent/'baseline/masks/manifest.json');pins=report['pins'].copy();rows=[]
 for t in [12,13,14]:
  p=B/f'tree{t}-bark-proposal-v1/proposed-bark.png';owned=np.asarray(Image.open(p))>0;pins[str(p)]=sha(p);left=owned&~mask;row=dict(tree=t,retained_bark_pixels=int(left.sum()),reservations=[]);reserved=np.zeros_like(mask)
  for mi in [35,76,107]:
   p=B.parent/f'baseline/masks/{mi:06}.png';a=np.array(Image.open(p))>0;pins[str(p)]=sha(p);x,y=manifest['masks'][mi]['box_top_left'];domain=np.zeros_like(mask);domain[y:y+a.shape[0],x:x+a.shape[1]]=a;reserved|=domain;row['reservations'].append(dict(mask=mi,pixels=int((left&domain).sum())))
  assert not (left&~reserved).any();row['all_retained_bark_explicitly_reserved']=True
  p=B/f'tree{t}-canopy-fragment-source-v1/scope.json'
  if p.exists():
   box=read(p)['absolute_interval'];pins[str(p)]=sha(p);union=np.zeros_like(mask)
   for f in sorted(p.parent.glob('???.png')):
    a=np.array(Image.open(f).convert('RGBA'))[:,:,3]>0;union[box[1]:box[3],box[0]:box[2]]|=a;pins[str(f)]=sha(f)
   row['dynamic_frame_union']=dict(pixels=int(union.sum()),filled=int((union&mask).sum()),protected=int((union&~mask).sum()),protected_outside_reservations=int((union&~mask&~reserved).sum()));assert not (union&~mask&~reserved).any()
  rows.append(row)
 static=read(B/'static-pair-publication-audit-v1/static-body-v1/receipt.json');static_models=[]
 for row in static['rows']:
  p=Path(row['model']);assert sha(p)==row['model_sha256'];static_models.append(dict(tree=row['tree'],model=str(p),model_sha256=sha(p),role='wood plus static-native-samples only; dynamic-frame0 role excluded; same approved full-asset pivot',native_check='Still required after role exclusion; structural payload preservation alone is insufficient'))
 plan=dict(status='PRIVATE GROUND INTEGRATION READY FOR JOINT REVIEW; publication remains HOLD',ground=dict(model=str(O/'model.glb'),model_sha256=report['model_sha256'],descriptor=str(O/'asset.json'),descriptor_sha256=sha(O/'asset.json'),approved_atlas_sha256=report['atlas_sha256'],known_rgba_and_alpha_exact=True,geometry_and_uv_buffers_exact=True,gameplay_exact=True),ownership=report['ownership'],reserved_domains=rows,static_candidates=static_models,animation_policy='Keep complete native Arbre06, its all-frame imagery/timing and every neighboring domain unchanged. Do not install full frame0 crown GLBs as static scenery beside it. Retain separately approved static-native sample geometry for Tree12/14.',ordered_next_steps=['Verify compact terrain GLB in production loader: source atlas equals approved fill, existing sampling/orientation/UV and source camera unchanged.','Audit static-only Tree12/14 native first hits after removal of frame0 role, retain full-role approval provenance.','Resolve Tree13 receiver and static75 ownership against existing approved source/bark export; do not omit it from trio placement.','Joint source and opposite oblique review of ground plus static tree bodies and reserved fern35/76 context; demonstrate no duplicate painted foliage or revealed holes.','Verify complete native Arbre06 runtime owner and all 14 phase coverage/timing with static bodies and filled receiver; no inferred ownership assignment.','Rebase prospective descriptor gameplay metadata against current map/catalog, derive lossy/previews and require canonical-sized source guards.','Root reviews final staged map and browser evidence, then builds shared-lock guarded publication with exact current pins and rollback.'],remaining_blockers=['Tree13 scoped static ownership/export integration is not included in the Tree12/14 static-body plan.','Role-separated static model first-hit and production renderer checks are pending.','Complete native animation owner/playback in staged editor/game contracts has not been demonstrated by this ground approval.','Tree10/11 have zero approved pixels in this receiver change and remain separate ownership work.'],pins=pins,canonical_writes=False)
 (O/'integration-plan.json').write_text(json.dumps(plan,indent=2)+'\n');print(json.dumps(dict(rows=rows,plan=str(O/'integration-plan.json')),indent=2))
if __name__=='__main__':main()
