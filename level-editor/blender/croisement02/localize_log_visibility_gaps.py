"""Localize immutable endpoint visibility residuals without changing the proof."""
import json
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw
from scipy.ndimage import label, binary_erosion
from catalog import OUT
from native_log_foreground_reference import sha


def main():
    proof=OUT/'log-state-appearance-proof-v4'
    root=OUT/'state-target-evidence/log-trap'
    dest=root/'visibility-gap-localization-v2'
    dest.mkdir(exist_ok=False)
    bindings={str(p.relative_to(OUT)):sha(p) for p in proof.iterdir() if p.is_file()}
    manifest=json.loads((proof/'manifest.json').read_text())
    box=manifest['camera']['bbox'];w=box[2]-box[0];h=box[3]-box[1]
    yy,xx=np.mgrid[:512,:512];scale=manifest['camera']['ortho_scale']
    ix=np.floor(w/2+(xx+.5-256)*scale/512).astype(int)
    iy=np.floor(h/2+(yy+.5-256)*scale/512).astype(int)
    valid=(ix>=0)&(ix<w)&(iy>=0)&(iy<h)
    native=np.array(Image.open(root/'native-order-reference/visible-log-phase0.png'))>0
    expected=np.zeros((512,512),bool);expected[valid]=native[iy[valid],ix[valid]]
    def magenta(name):
        p=np.array(Image.open(proof/name));return (p[:,:,0]>240)&(p[:,:,1]<15)&(p[:,:,2]>240)
    body=magenta('logs-only-visibility.png');actual=magenta('applied-phase-00-visibility.png')
    survey_path=root/'applied-sloped-bank-hypothesis-v2.json'
    axes=np.array(json.loads(survey_path.read_text())['survey'])
    source=Image.open(root/'tick-089.png').convert('RGBA')
    canvas=Image.new('RGBA',source.size,(45,45,45,255));canvas.alpha_composite(source)
    canvas=canvas.convert('RGB');draw=ImageDraw.Draw(canvas)
    report={}
    for kind,mask,color in [('missing_geometry',expected&~body&~actual,(255,0,230)),('still_hidden',expected&body&~actual,(255,40,20))]:
        counts=np.zeros((h,w),np.uint16);np.add.at(counts,(iy[mask],ix[mask]),1)
        Image.fromarray(counts).save(dest/f'{kind}-native-counts.png')
        labels,n=label(counts>0,np.ones((3,3)));regions=[]
        for i in range(1,n+1):
            ys,xs=np.nonzero(labels==i);points=np.column_stack((xs,ys));dist=[]
            for ax,ay,bx,by,r,*_ in axes:
                a=np.array([ax,ay]);d=np.array([bx-ax,by-ay]);t=np.clip((points-a)@d/(d@d),0,1)
                dist.append(np.mean(np.linalg.norm(points-(a+t[:,None]*d),axis=1)-r))
            nearest=int(np.argmin(dist));samples=int(counts[ys,xs].sum())
            regions.append(dict(samples=samples,native_pixels=len(xs),native_bbox=[int(xs.min()+box[0]),int(ys.min()+box[1]),int(xs.max()+box[0]+1),int(ys.max()+box[1]+1)],nearest_axis=nearest,mean_axis_edge_distance=float(dist[nearest])))
            for x,y in zip(xs,ys):draw.point((int(x),int(y)),fill=color)
        regions.sort(key=lambda r:-r['samples'])
        report[kind]=dict(render_samples=int(mask.sum()),native_pixels=int((counts>0).sum()),samples_more_than_one_pixel_inside_expected=int((mask&binary_erosion(expected,iterations=1)).sum()),samples_more_than_two_pixels_inside_expected=int((mask&binary_erosion(expected,iterations=2)).sum()),regions=regions)
    for i,(ax,ay,bx,by,*_) in enumerate(axes):
        draw.line((ax,ay,bx,by),fill=(50,150,255));draw.text((ax,ay),str(i),fill=(255,255,255))
    canvas.resize((w*4,h*4),Image.Resampling.NEAREST).save(dest/'source-residuals-and-axes.png')
    assert report['missing_geometry']['render_samples']==1546
    assert report['still_hidden']['render_samples']==348
    assert bindings=={str(p.relative_to(OUT)):sha(p) for p in proof.iterdir() if p.is_file()}
    result=dict(status='localization only; nearest axis is a diagnostic assignment, not physical identity',proof_files=bindings,source_sha256=sha(root/'tick-089.png'),survey_sha256=sha(survey_path),residuals=report)
    (dest/'manifest.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps({k:{**v,'regions':v['regions'][:12]} for k,v in report.items()},indent=2))

if __name__=='__main__':main()
