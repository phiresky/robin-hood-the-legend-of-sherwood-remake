"""Fit tube width and vertical source placement within explicit trace uncertainty."""
import json,math
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
from scipy.optimize import differential_evolution
ROOT=Path(__file__).resolve().parents[3];OUT=ROOT/'level-editor/work/croisement03-refinement/restart2'
RINGS=[(574,919,13),(595,914,13),(615,909,12),(640,900,12),(663,889,10),(687,881,8),(713,878,7),(738,875,6),(752,871,5),(764,864,4),(769,860,2.5)]
SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35));CROP=(570,845,780,935)
EXPECTED=np.asarray(Image.open(OUT/'fallen-log-v1/observed-domain.png').crop(CROP))>0
MEASURE=np.ones(EXPECTED.shape,dtype=bool);MEASURE[:,:592-CROP[0]]=False

def geometry(parameters):
    shift,scale,tip=parameters;centers=[];radii=[]
    for i,(x,y,r) in enumerate(RINGS):
        radius=r*scale if i<8 else r*scale+tip*(i-7)/3;radii.append(radius);z=max(radius,8);centers.append(np.array([x,-(y+shift+z*COS)/SIN,z]))
    vertices=[];faces=[];sides=20
    for i,(c,r) in enumerate(zip(centers,radii)):
        axis=centers[min(i+1,len(centers)-1)]-centers[max(i-1,0)];axis/=np.linalg.norm(axis);side=np.cross(axis,[0,0,1]);side/=np.linalg.norm(side);up=np.cross(axis,side)
        for j in range(sides):t=math.tau*j/sides;vertices.append(c+r*(math.cos(t)*side+math.sin(t)*up))
    for i in range(len(RINGS)-1):
        for j in range(sides):faces.append([i*sides+j,i*sides+(j+1)%sides,(i+1)*sides+(j+1)%sides,(i+1)*sides+j])
    faces += [list(reversed(range(sides))),[(len(RINGS)-1)*sides+j for j in range(sides)]]
    return np.array(vertices),faces

def render(parameters):
    vertices,faces=geometry(parameters);screen=np.column_stack((vertices[:,0]-CROP[0],-vertices[:,1]*SIN-vertices[:,2]*COS-CROP[1]));image=Image.new('L',(CROP[2]-CROP[0],CROP[3]-CROP[1]));draw=ImageDraw.Draw(image)
    for face in faces:draw.polygon([tuple(screen[i]) for i in face],fill=255)
    return np.asarray(image)>0

def loss(p):
    result=render(p);return np.count_nonzero(EXPECTED&~result&MEASURE)*2+np.count_nonzero(result&~EXPECTED&MEASURE)

def main():
    target=OUT/'fallen-log-fit-v2';target.mkdir(exist_ok=False);fit=differential_evolution(loss,[(0,4),(.6,1.05),(0,3)],seed=31,popsize=12,maxiter=90,polish=False);vertices,faces=geometry(fit.x)
    result=render(fit.x);record=dict(parameters=fit.x.tolist(),vertices=vertices.tolist(),faces=faces,measured_crop=list(CROP),missing=int(np.count_nonzero(EXPECTED&~result&MEASURE)),extra=int(np.count_nonzero(result&~EXPECTED&MEASURE)),scope='Source-trace occupancy fit only. Inferred left extension excluded from penalty. Actual native render and semantic source review remain mandatory.')
    (target/'fit.json').write_text(json.dumps(record,indent=2)+'\n');print({k:v for k,v in record.items() if k not in ['vertices','faces']})
if __name__=='__main__':main()
