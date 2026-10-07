"""Measure repeating native chain width before changing physical link spacing."""
import hashlib,json,math
from pathlib import Path
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement';SOURCE=WORK/'geometry-pass-01/native-state-source-v1';OUT=WORK/'restart2/winch-native-link-period-v1.json'
if OUT.exists():raise FileExistsError(OUT)
record=next(r for r in json.loads((SOURCE/'manifest.json').read_text())['records'] if r['id']=='patch-004');frames=next(r['frames'] for r in record['rows'] if r['action']=='PatchTransition');poses=json.loads((WORK/'restart2/winch-motion-physical-v2/motion.json').read_text())['rows'];periods=np.linspace(6.5,8,121);rows=[]
for index,(frame,pose) in enumerate(zip(frames,poses)):
    im=np.asarray(Image.open(SOURCE/frame['image']).convert('RGBA'));sides={}
    for side,lo,hi in [('left',2394,2405),('right',2406,2415)]:
        samples=[]
        for y in range(884,939):
            if side=='left' and abs(y+.5-pose['screen_center_y'])<12:continue
            yy=y-frame['bbox'][1]
            count=sum(float(im[yy,x-frame['bbox'][0],3])/255 for x in range(lo,hi) if 0<=yy<im.shape[0] and 0<=x-frame['bbox'][0]<im.shape[1]);samples.append((y,count))
        values=np.asarray(samples);y=values[:,0];observed=values[:,1];variance=float(np.sum((observed-observed.mean())**2));scores=[]
        for period in periods:
            angle=y*math.tau/period;design=np.column_stack((np.ones(len(y)),np.cos(angle),np.sin(angle),np.cos(2*angle),np.sin(2*angle)));coeff=np.linalg.lstsq(design,observed,rcond=None)[0];residual=float(np.sum((observed-design@coeff)**2));scores.append({'period_native_pixels':float(period),'explained_variance':1-residual/variance if variance>1e-9 else 0})
        scores.sort(key=lambda r:-r['explained_variance']);sides[side]={'sample_rows':len(y),'best':scores[0],'scores':scores}
    rows.append({'frame':index,'sides':sides})
summary={}
for side in ('left','right'):
    scores={float(p):[] for p in periods}
    for row in rows:
        for score in row['sides'][side]['scores']:scores[score['period_native_pixels']].append(score['explained_variance'])
    aggregate=[{'period_native_pixels':period,'mean_explained_variance':float(np.mean(values))} for period,values in scores.items()];aggregate.sort(key=lambda r:-r['mean_explained_variance']);summary[side]={'best_global_period':aggregate[0],'period7_baseline':next(r for r in aggregate if abs(r['period_native_pixels']-7)<1e-8),'top10':aggregate[:10]}
result={'status':'Source-only spacing diagnostic; no geometry changed','method':'Each frame alpha-row width is fitted to two Fourier harmonics with free phase and amplitude, periods6.5..8.0px. Travelling-part rows excluded. Aggregate scores average all45frames.','source_hashes':[hashlib.sha256((SOURCE/f['image']).read_bytes()).hexdigest() for f in frames],'summary':summary,'frames':rows,'limitations':['Rasterized edge changes and oblique link planes may bias period estimates.','This does not prove uniform physical link spacing or a material link identity.','A physical spacing change requires repeating contact/source review and must not alter approved timber.']};OUT.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(summary,indent=2))
