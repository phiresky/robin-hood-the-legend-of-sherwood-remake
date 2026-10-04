"""Fit a closed stump's section profile to its separately reviewed wood domain."""
import json
import math
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
from scipy.optimize import differential_evolution
from catalog import OUT

SIN,COS=math.sin(math.radians(35)),math.cos(math.radians(35))
CAP=np.array([(578,654),(586,653),(594,655),(600,659),(601,664),(598,669),
              (591,673),(582,674),(574,672),(570,668),(570,662),(573,658)],float)
Z=27.001001/COS
UPPER=np.column_stack([CAP[:,0],-(CAP[:,1]+Z*COS)/SIN,np.full(len(CAP),Z)])
CENTER=UPPER.mean(axis=0)


def surface(parameters):
    x,y,r0,r1,r2=parameters
    base=np.array([x,-y/SIN,0.])
    rings=[]
    for t,r in [(0,r0),(.2,r1),(.6,r2),(1,1.)]:
        center=base*(1-t)+CENTER*t
        ring=center+(UPPER-CENTER)*r;ring[:,2]=Z*t;rings.append(ring)
    return np.concatenate(rings)


def silhouette(parameters):
    vertices=surface(parameters);screen=np.column_stack([vertices[:,0]-558,-vertices[:,1]*SIN-vertices[:,2]*COS-653])
    image=Image.new('L',(54,74));draw=ImageDraw.Draw(image);n=len(CAP)
    for j in range(3):
        for i in range(n):
            draw.polygon([tuple(screen[k]) for k in [j*n+i,j*n+(i+1)%n,(j+1)*n+(i+1)%n,(j+1)*n+i]],fill=255)
    draw.polygon([tuple(p) for p in screen[-n:]],fill=255)
    return np.asarray(image)>0


def main():
    destination=OUT/'stump65-profile-fit-v1';destination.mkdir(exist_ok=False)
    target=np.asarray(Image.open(OUT/'stump65-source-split-v3/wood-domain.png'))>0
    def objective(p):
        visible=silhouette(p)
        # The main term is measured source silhouette, with a modest penalty
        # for aggressive hidden taper or a shifted ground anchor.
        return float(np.count_nonzero(visible^target)+.08*((p[0]-587)**2+(p[1]-695)**2)+8*((p[2]-.8)**2+(p[3]-.9)**2+(p[4]-.95)**2))
    initial=[583,695,.78,.80,.90]
    result=differential_evolution(objective,[(582,594),(690,705),(.55,1.15),(.65,1.15),(.75,1.15)],seed=65,popsize=12,maxiter=100,polish=False)
    rows=[]
    for name,p in [('initial',initial),('fitted',result.x.tolist())]:
        visible=silhouette(p);Image.fromarray(visible.astype('uint8')*255).save(destination/(name+'.png'))
        intersection=int((visible&target).sum());union=int((visible|target).sum())
        rows.append(dict(name=name,parameters=p,intersection_over_union=intersection/union,
                         missing_pixels=int((target&~visible).sum()),extra_pixels=int((visible&~target).sum())))
    (destination/'fit.json').write_text(json.dumps(dict(rows=rows,cap_pixels=CAP.tolist(),
        target='stump65-source-split-v3/wood-domain.png',status='source construction fit; not independent visual validation',
        interpretation='Surrounding field grass is retained by terrain. It does not establish a separate tall tuft.'),indent=2)+'\n')
    print(json.dumps(rows))


if __name__=='__main__':main()
