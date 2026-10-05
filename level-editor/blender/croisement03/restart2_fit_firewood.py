"""Fit a compact three-billet hypothesis to native timber silhouette114."""
import hashlib,json,math
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
from scipy.optimize import differential_evolution
from scipy.spatial import ConvexHull
ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/croisement03-refinement/restart2/firewood-fit-v1'
BASE=OUT.parents[1]/'baseline'
SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35))

def geometry(parameters):
    cx,cy,angle,length,radius=parameters
    u=np.array([math.cos(angle),math.sin(angle),0]);v=np.array([-u[1],u[0],0]);up=np.array([0,0,1]);c=np.array([cx,-cy/SIN,0.])
    verts=[];faces=[];logs=[]
    for row,count in enumerate([2,1]):
        for col in range(count):
            center=c+v*((col-(count-1)/2)*radius*1.95)+up*(radius+row*radius*1.7)
            a=center-u*length/2;b=center+u*length/2;n=len(verts);segments=16
            for end in [a,b]:
                verts.extend((end+radius*(v*math.cos(j*math.tau/segments)+up*math.sin(j*math.tau/segments))).tolist() for j in range(segments))
            faces.append(tuple(n+j for j in reversed(range(segments))))
            faces.extend((n+j,n+(j+1)%segments,n+segments+(j+1)%segments,n+segments+j) for j in range(segments));faces.append(tuple(n+segments+j for j in range(segments)))
            logs.append(dict(a=a.tolist(),b=b.tolist(),radius=radius))
    return np.array(verts),faces,logs

def raster(parameters):
    vertices,_,_=geometry(parameters);xy=np.c_[vertices[:,0],-vertices[:,1]*SIN-vertices[:,2]*COS]
    image=Image.new('1',(360,240));draw=ImageDraw.Draw(image)
    for i in range(3):
        points=(xy[i*32:(i+1)*32]-[385,740])*4
        hull=ConvexHull(points);draw.polygon([tuple(p) for p in points[hull.vertices]],fill=1)
    return np.asarray(image.resize((90,60),Image.Resampling.NEAREST))

def main():
    OUT.mkdir(parents=True,exist_ok=False)
    expected=Image.new('L',(90,60));expected.paste(Image.open(BASE/'masks/000114.png'),(397-385,749-740));mask=np.array(expected)>0
    def score(parameters):
        hit=raster(parameters);return 1-np.count_nonzero(hit&mask)/np.count_nonzero(hit|mask)
    fit=differential_evolution(score,[(423,434),(773,787),(-.90,-.60),(73,95),(3,6)],seed=3103,popsize=12,maxiter=100,tol=.0001)
    vertices,faces,logs=geometry(fit.x);hit=raster(fit.x)
    result=dict(parameters=fit.x.tolist(),vertices=vertices.tolist(),faces=faces,logs=logs,source_mask_sha256=hashlib.sha256((BASE/'masks/000114.png').read_bytes()).hexdigest(),source_fit=dict(iou=1-float(fit.fun),missing=int(np.count_nonzero(mask&~hit)),extra=int(np.count_nonzero(hit&~mask))),limitations=['Three billets are a compact construction hypothesis; the obscured source does not prove a complete log count.','Fit uses native occupancy silhouette, not proof of visible RGB ownership.','Depth follows a round log cross-section; reverse faces are inferred.'])
    (OUT/'fit.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result['source_fit']))
    source=Image.open(BASE/'covered.png').convert('RGB').crop((385,740,475,800));pixels=np.array(source);pixels[mask&~hit]=[255,40,40];pixels[hit&~mask]=[0,220,255];Image.fromarray(pixels).resize((900,600),Image.Resampling.NEAREST).save(OUT/'silhouette-fit.png')

if __name__=='__main__':main()
