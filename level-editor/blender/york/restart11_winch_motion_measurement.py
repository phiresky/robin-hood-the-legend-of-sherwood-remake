"""Measure the travelling part from its unobstructed left edge in all source frames."""
import hashlib,json
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement';SRC=WORK/'geometry-pass-01/native-state-source-v1';OUT=WORK/'restart2/winch-motion-measurement-v2'
if OUT.exists():raise FileExistsError(OUT)
record=next(r for r in json.loads((SRC/'manifest.json').read_text())['records'] if r['id']=='patch-004');frames=next(r['frames'] for r in record['rows'] if r['action']=='PatchTransition');assert len(frames)==45
OUT.mkdir();rows=[];sheets=[Image.new('RGB',(650,960),'#303840') for _ in range(3)]
for i,f in enumerate(frames):
 p=SRC/f['image'];assert hashlib.sha256(p.read_bytes()).hexdigest()==f['sha256'];im=Image.open(p).convert('RGBA');a=np.asarray(im);x,y,w,h=f['bbox'];xx=np.arange(w)[None,:]+x;yy=np.arange(h)[:,None]+y
 # Left exposed arc avoids both chain columns and the lower crank. Its extrema
 # estimate vertical centre only after the whole left arc is below the lintel.
 chosen=(a[:,:,3]>127)&(xx<2395)&(yy<941);ys,xs=np.where(chosen);bounds=None if len(xs)<3 else [int(xs.min()+x),int(ys.min()+y),int(xs.max()+x+1),int(ys.max()+y+1)]
 center=None if bounds is None or bounds[1]<=882 else (bounds[1]+bounds[3]-1)/2
 rows.append({'frame':i,'sha256':f['sha256'],'delay':f['delay'],'left_arc_bbox':bounds,'observed_center_y':center,'center_uncertainty_native_pixels':1.5 if center is not None else None,'linear_endpoint_candidate_center_y':853+72*i/44})
 tile=Image.new('RGBA',(42,98),'#303840');tile.alpha_composite(im,(x-2389,y-882));d=ImageDraw.Draw(tile)
 if center is not None:d.line([(0,center-882),(20,center-882)],fill='#ff66dd',width=1)
 sheet=sheets[i//15];sx=(i%5)*130;sy=((i%15)//5)*320;sheet.paste(tile.resize((126,294),Image.Resampling.NEAREST).convert('RGB'),(sx,sy+24));ImageDraw.Draw(sheet).text((sx+3,sy+5),f'Frame {i}',fill='white')
for i,sheet in enumerate(sheets):sheet.save(OUT/f'source-frames-{i*15:02d}-{i*15+14:02d}.png')
fitrows=[r for r in rows if 22<=r['frame']<=35];coef=np.polyfit([r['frame'] for r in fitrows],[r['observed_center_y'] for r in fitrows],2);residual=[float(np.polyval(coef,r['frame'])-r['observed_center_y']) for r in fitrows]
report={'status':'Source screen-motion measurement only; no runtime or geometry approval','scope':'Unclipped left arc of travelling round part, all45 source hashes checked. Pink line is measured screen centre, not a 3D anchor.','rows':rows,'descent_fit_diagnostic':{'frames':[22,35],'quadratic_descending_coefficients':coef.tolist(),'max_abs_residual_pixels':max(abs(x) for x in residual),'not_an_animation_contract':True},'findings':['Visible round part accelerates downward until about frame36, then rebounds and settles; a linear endpoint interpolation does not reproduce it.','Frames0–21 have no complete observed left arc. Early inferred positions must remain behind the actual lintel without cropping reusable geometry.','Post-frame35 measurements intentionally use only left-arc pixels; the prior both-side bbox sometimes included lower crank pixels.','This measurement does not establish chain translation or rotating-crank winding count. Those need independent source fitting.'],'source_manifest_sha256':hashlib.sha256((SRC/'manifest.json').read_bytes()).hexdigest()}
(OUT/'measurement.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report['descent_fit_diagnostic']))
