"""Build source-supported dense motion fields for a named remaining canopy loop."""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
os.environ.setdefault('OMP_NUM_THREADS','1')
import argparse
import hashlib
import json
from pathlib import Path
import numpy as np
from PIL import Image
from scipy.ndimage import gaussian_filter
from restart14_tree42_dense_motion import solve, warp
from forest_layout import CROWNS

BASE=Path(__file__).resolve().parents[2]/'work/croisement02-refinement'
MANIFEST=BASE/'animation-references/manifest.json'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--animation',type=int,required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args();assert args.animation in CROWNS and args.animation!=1
    args.output.mkdir(exist_ok=False)
    group=next(a for a in json.loads(MANIFEST.read_text())['animations'] if a['index']==args.animation)
    left=min(f['bbox'][0] for f in group['frames'])-16;top=min(f['bbox'][1] for f in group['frames'])-16
    right=max(f['bbox'][0]+f['bbox'][2] for f in group['frames'])+16;bottom=max(f['bbox'][1]+f['bbox'][3] for f in group['frames'])+16
    h,w=bottom-top,right-left;frames=[];pins=[]
    for f in group['frames']:
        a=np.asarray(Image.open(f['image']).convert('RGBA'));x,y,fw,fh=f['bbox'];assert a.shape==(fh,fw,4)
        canvas=np.zeros((h,w,4),np.float32);canvas[y-top:y-top+fh,x-left:x-left+fw]=a/255
        frames.append(canvas);pins.append(dict(path=f['image'],sha256=sha(Path(f['image'])),bbox=f['bbox']))
    features=[gaussian_filter(np.concatenate((a[:,:,:3]*a[:,:,3:],a[:,:,3:]),axis=2),(2.,2.,0)) for a in frames]
    support=gaussian_filter(frames[0][:,:,3],2)>.08;fields=[];rows=[];cache={}
    for phase in range(14):
        key=hashlib.sha256(frames[phase].tobytes()).hexdigest()
        if key in cache:
            field,prior=cache[key];row={**prior,'phase':phase,'identical_source_phase':prior['phase']}
        else:
            forward=solve(features[0],features[phase]) if phase else np.zeros((h,w,2),np.float32)
            backward=solve(features[phase],features[0]) if phase else forward.copy()
            roundtrip=np.linalg.norm(forward+warp(backward,forward),axis=2)
            error=np.mean((features[0]-warp(features[phase],forward))**2,axis=2)
            zero=np.mean((features[0]-features[phase])**2,axis=2)
            confidence=support&(roundtrip<.8)&(error<=zero+.0004)
            field=(forward*gaussian_filter(confidence.astype('float32'),1)[:,:,None]).astype('float32')
            row=dict(phase=phase,supported_pixels=int(support.sum()),consistent_pixels=int(confidence.sum()),maximum_motion=float(np.linalg.norm(field,axis=2).max()),mean_motion=float(np.linalg.norm(field[support],axis=1).mean()),mean_roundtrip=float(roundtrip[support].mean()),fitted_error=float(error[support].mean()),stationary_error=float(zero[support].mean()))
            cache[key]=(field,row)
        fields.append(field);rows.append(row);print(json.dumps(row),flush=True)
        np.savez_compressed(args.output/f'phase-{phase:02}.npz',flow=field)
    np.savez_compressed(args.output/'flows.npz',flow=np.stack(fields),bbox=np.array([left,top,w,h]))
    report=dict(status='PRIVATE_DENSE_MOTION_FIELDS_NOT_GEOMETRY_REVIEW',animation_index=args.animation,crown_masks=[r[0] for r in CROWNS[args.animation]],bbox=[left,top,w,h],source_frames=pins,manifest_sha256=sha(MANIFEST),recipe_sha256=sha(Path(__file__)),solver_sha256=sha(Path(__file__).with_name('restart14_tree42_dense_motion.py')),rows=rows,cycle_ticks=56,limits=['Bidirectional source-plane motion with photometric non-regression; hidden depth is not measured.','Repeated phases share fields only after exact full aligned RGBA equality.','Dense interpolation and crown ownership need physical model/contact review before approval.','No texture synthesis, canonical writes or runtime edits.'])
    (args.output/'report.json').write_text(json.dumps(report,indent=2)+'\n')


if __name__=='__main__':main()
