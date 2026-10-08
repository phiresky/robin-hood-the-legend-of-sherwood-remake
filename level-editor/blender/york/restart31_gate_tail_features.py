"""Compare late exposed timber tips separately from the ambiguous upper sliver."""
import json
from pathlib import Path
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement';SOURCE=WORK/'geometry-pass-01/native-state-source-v1';OUT=WORK/'restart2/gate-crossbar-tracking-v1/tail-features.json'
j=json.loads((SOURCE/'manifest.json').read_text());frames=next(r['frames']for r in next(x for x in j['records']if x['id']=='patch-000')['rows']if r['action']=='PatchTransition');images=[]
for f in frames:
 c=Image.new('RGBA',(11,71));c.alpha_composite(Image.open(SOURCE/f['image']).convert('RGBA'));images.append(np.asarray(c,dtype=float))
ref=images[44];rows=[]
for fi in range(30,45):
 target=images[fi];observed=(target[:,:,3]>0);observed[:6]=False;scores=[]
 for shift in range(-30,15):
  shifted=np.zeros_like(ref)
  if shift>=0:shifted[shift:]=ref[:71-shift]
  else:shifted[:71+shift]=ref[-shift:]
  present=shifted[:,:,3]>0;common=observed&present;missing=observed&~present;union=(observed|present);union[:6]=False
  error=float(np.abs(target[:,:,:3]-shifted[:,:,:3])[common].mean())if common.any()else None
  loss=(float(np.abs(target[:,:,:3]-shifted[:,:,:3])[common].sum()/3)+int(missing.sum())*255)/max(1,int(observed.sum()))
  scores.append({'shift_down_from44':shift,'observed_tip_pixels':int(observed.sum()),'common_pixels':int(common.sum()),'unmatched_observed_pixels':int(missing.sum()),'rgb_mean_error':error,'loss':loss,'overlap_iou':float(common.sum()/max(1,union.sum()))})
 ranked=sorted(scores,key=lambda x:(x['loss'],-x['overlap_iou']));rows.append({'frame':fi,'best':ranked[:4],'conclusion':'NO_EXPOSED_TIP_FEATURE'if not observed.any()else'FEATURE_MATCH_PROPOSAL'})
report={'status':'CPU_TAIL_RGB_ALPHA_MATCH','reference':44,'excluded_top_rows':6,'reason':'Upper six-row sliver is repeated/ambiguous and cannot determine unique gate displacement. Track only exposed lower timber tips.','rows':rows,'limitation':'No feature confidence can be assigned when no exposed tip remains; frame36 upper sliver cannot prove65pixel lift.'};OUT.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps([{'frame':r['frame'],'best':r['best'][0]}for r in rows]))
