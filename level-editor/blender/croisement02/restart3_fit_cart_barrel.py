"""Fit a bounded lying-cask hypothesis to the terminal native cargo silhouette."""
import json
import math
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
from scipy.optimize import differential_evolution
from catalog import OUT

SIN,COS=math.sin(math.radians(35)),math.cos(math.radians(35))


def vertices(parameters):
    angle,length,radius,cx,cy=parameters
    axis=np.array([math.cos(angle),math.sin(angle),0.])
    cross=np.array([-axis[1],axis[0],0.])
    center=np.array([cx,-(cy+radius*COS)/SIN,radius])
    rings=[]
    for t,scale in [(-.5,.88),(-.4,.94),(-.28,1),(-.05,1),(.18,1),(.38,.94),(.5,.88)]:
        rings.append([list(center+axis*(length*t)+radius*scale*(cross*math.cos(a)+np.array([0,0,math.sin(a)])))
                      for a in np.arange(32)*math.tau/32])
    return np.array(rings)


def silhouette(parameters):
    pts=vertices(parameters)
    image=Image.new('L',(64,125));draw=ImageDraw.Draw(image)
    project=lambda p:(float(p[0]),float(-p[1]*SIN-p[2]*COS))
    for ring in [pts[0],pts[-1]]:draw.polygon([project(p) for p in ring],fill=255)
    for i in range(len(pts)-1):
        for j in range(32):draw.polygon([project(p) for p in [pts[i,j],pts[i,(j+1)%32],pts[i+1,(j+1)%32],pts[i+1,j]]],fill=255)
    return np.asarray(image)>0


def main():
    import hashlib
    source_manifest=OUT/'state-target-evidence/south-cart/manifest.json'
    part=json.loads(source_manifest.read_text())['parts'][2];frame=part['frames'][-1]
    rgba=np.asarray(Image.open(frame['image']).convert('RGBA'))
    expected=rgba[:,:,3]>127
    expected[:68]=False
    # The persistent narrow vertical strip has no established volumetric role.
    expected[68:75,29:33]=False
    objective=lambda p:int(np.logical_xor(silhouette(p),expected).sum())
    result=differential_evolution(objective,[(.7,1.15),(42,76),(10,18),(25,39),(90,106)],
                                  seed=91235,maxiter=60,popsize=10,polish=False)
    mask=silhouette(result.x);dest=OUT/'restart3-south-cart/barrel-fit-v1';dest.mkdir(exist_ok=False)
    diagnostic=rgba.copy();diagnostic[expected&~mask]=[255,30,180,255];diagnostic[mask&~expected]=[20,180,255,255]
    Image.fromarray(diagnostic).resize((384,750),Image.Resampling.NEAREST).save(dest/'silhouette-comparison.png')
    Image.fromarray(np.uint8(expected)*255).save(dest/'body-domain.png')
    report=dict(status='Private bounded lying banded-cask hypothesis; no geometry approval',parameters=result.x.tolist(),
        source_frame=frame,source_manifest_sha256=hashlib.sha256(source_manifest.read_bytes()).hexdigest(),
        global_origin=[part['position'][i]+frame['offset'][i] for i in range(2)],
        expected_pixels=int(expected.sum()),missing_pixels=int((expected&~mask).sum()),extra_pixels=int((mask&~expected).sum()),
        silhouette_iou=float((mask&expected).sum()/(mask|expected).sum()),
        limitations=['Persistent thin vertical native strip reserved for native presentation, not physical cask geometry.',
                    'Single banded cask is a bounded hypothesis; native metadata does not establish cargo count/material.',
                    'Ground contact and finite hoops require saved-model review; no animation or state behavior changed.'])
    (dest/'fit.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:report[k] for k in ['parameters','silhouette_iou','missing_pixels','extra_pixels']}))


if __name__=='__main__':main()
