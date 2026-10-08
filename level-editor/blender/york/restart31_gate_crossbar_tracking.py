"""Track native gate timber features independently of sprite bounding height."""
import json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement';S=WORK/'geometry-pass-01/native-state-source-v1';OUT=WORK/'restart2/gate-crossbar-tracking-v1/tracking.json'
if OUT.exists():raise FileExistsError(OUT)
manifest=json.loads((S/'manifest.json').read_text());frames=next(row['frames']for row in next(r for r in manifest['records']if r['id']=='patch-000')['rows']if row['action']=='PatchTransition')
images=[]
for f in frames:
 p=S/f['image'];assert hashlib.sha256(p.read_bytes()).hexdigest()==f['sha256'];im=Image.open(p).convert('RGBA');canvas=Image.new('RGBA',(11,71));canvas.alpha_composite(im,(f['bbox'][0]-2343,f['bbox'][1]-868));images.append(np.array(canvas,dtype=float))
base=images[0];rows=[]
for fi,target in enumerate(images):
 h=frames[fi]['bbox'][3];scores=[]
 for dy in range(71):
  shifted=np.zeros_like(base);shifted[:71-dy]=base[dy:];ta=target[:,:,3]/255;sa=shifted[:,:,3]/255
  rgb=np.abs(target[:,:,:3]*ta[:,:,None]-shifted[:,:,:3]*sa[:,:,None]).mean(axis=2);alpha=np.abs(ta-sa)*255
  # Exclude bottommost three rows: this fit cannot simply minimize bbox height.
  interior=np.zeros((71,11),bool);interior[1:max(2,h-3),2:10]=True
  score=float((rgb+alpha*.5)[interior].mean());full=float((rgb+alpha*.5).mean());scores.append({'shift_up_pixels':dy,'interior_score':score,'full_score':full,'exact_opaque_rgb':int(np.sum(np.all(target[:,:,:3]==shifted[:,:,:3],axis=2)&(ta>0)&(sa>0)))})
 best=sorted(scores,key=lambda q:(q['interior_score'],q['full_score']))[:4];global_best=min(scores,key=lambda q:q['full_score']);rows.append({'frame':fi,'height':h,'bbox_inferred_shift':71-h,'feature_fit':best,'full_fit':global_best})
report={'status':'CPU_NATIVE_TIMBER_FEATURE_TRACKING','reference_frame':0,'source_manifest_sha256':hashlib.sha256((S/'manifest.json').read_bytes()).hexdigest(),'method':'Compare shifted reference timber RGB and alpha in native columns2..9 and rows1..height-4; exclude bottom boundary. Enumerate0..70upward pixels; no resampling. Full-frame fit is a separate corroboration.','rows':rows,'limits':['Subpixel displacement and rasterized lighting differences remain uncertain.','Late frames contain very few timber pixels; report runner-up fits and counts rather than treating bounds as complete motion evidence.','No mesh, runtime, descriptor or approval changed.']};OUT.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps([{'frame':r['frame'],'bbox':r['bbox_inferred_shift'],'feature':r['feature_fit'][0]['shift_up_pixels'],'full':r['full_fit']['shift_up_pixels'],'error':r['feature_fit'][0]['interior_score']}for r in rows]))
