"""Freeze published canopy receivers and native temporal support for one prototype."""
from pathlib import Path
import hashlib,json
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/croisement02-refinement'
LIB=ROOT/'level-editor/library'
DEST=OUT/'restart14-canopy-animation/source-reconciliation-v1'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
read=lambda p:json.loads(p.read_text())
def main():
 DEST.mkdir(parents=True,exist_ok=False)
 audit_path=OUT/'restart10-ambient-source-audit-v1/report.json';audit=read(audit_path)
 map_path=LIB/'scenes/croisement02.rhlos-map.json';document=read(map_path)
 index=read(LIB/'3d-assets/index.json');entries={e['id']:e for e in index['assets']}
 groups=[];unions=[]
 for a in audit['animations'][:8]:
  frames=[];union=np.zeros((1152,1792),bool)
  for f in a['frames']:
   p=Path(f['file']);assert sha(p)==f['sha256'];im=np.asarray(Image.open(p).convert('RGBA'));x,y,w,h=f['bbox'];assert im.shape[:2]==(h,w)
   canvas=np.zeros_like(union);x0=max(0,x);y0=max(0,y);x1=min(1792,x+w);y1=min(1152,y+h)
   canvas[y0:y1,x0:x1]=im[y0-y:y1-y,x0-x:x1-x,3]>0;union|=canvas
   frames.append({'path':str(p),'sha256':sha(p),'bbox':f['bbox'],'duration_ticks':f['duration_ticks'],'visible_pixels':int(canvas.sum())})
  receivers=[]
  for asset in a['source_associated_assets']:
   dp=LIB/'3d-assets'/entries[asset]['descriptor'];d=read(dp);mp=dp.parent/d['model']
   receivers.append({'id':asset,'descriptor':str(dp),'descriptor_sha256':sha(dp),'model':str(mp),'model_sha256':sha(mp),'placements':[p for p in document['placements']if asset in p['assets']],'parts':[p['node']for p in d['parts']]})
  groups.append({'index':a['index'],'profile':a['profile'],'frames':frames,'cycle_ticks':sum(f['duration_ticks']for f in frames),'receivers':receivers,'source_union_pixels':int(union.sum())});unions.append(union)
 representative=1;overlaps=[{'other_group':i,'temporal_union_overlap':int((unions[representative]&u).sum())}for i,u in enumerate(unions)if i!=representative]
 Image.fromarray(unions[representative].astype('uint8')*255).save(DEST/'tree42-temporal-union.png')
 report={'status':'RECONCILED_SOURCE_AND_PUBLISHED_PINS_ONLY','map':str(map_path),'map_sha256':sha(map_path),'source_audit_sha256':sha(audit_path),'groups':groups,'representative':{'group':1,'asset':'croisement02-tree-42','reason':'Single receiver association avoids splitting a shared sprite into several crowns for the first prototype.','other_temporal_groups':overlaps},'timing':{'frame_duration_ticks':4,'frames':14,'cycle_ticks':56,'clock_hz':25,'cycle_seconds':2.24,'behavior':'Advance only after counter exceeds serialized delay3; wrap after last frame. Frozen/inactive animation does not advance.'},'next_proof':['Bind actual visible canopy triangles and nearest physical hits for every native phase; adjacent static foreground ownership remains separate from sprite overlap.','Estimate locally coherent leaf-cluster motion from native frame correspondences with forward/backward consistency; discard ambiguous matches.','Apply a reversible private mesh deformation with unchanged material/UV topology and fixed wood, phase0 exact. Depth motion is unobserved and must remain constrained or explicitly inferred.','Render native first plus seven other physical views, phase extrema and loop seam; compare source support, contact and neighboring occlusion before propagating.'],'holds':['Source image centroids are not 3D trajectories.','No material frame swapping or billboards counts as physical canopy completion.','Tree21 has no observed sequence; fringe22 ownership remains unresolved.','Published pins are frozen inputs; no static, runtime or catalog mutation.']}
 (DEST/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(sha(DEST/'report.json'));print(json.dumps(report['representative']))
if __name__=='__main__':main()
