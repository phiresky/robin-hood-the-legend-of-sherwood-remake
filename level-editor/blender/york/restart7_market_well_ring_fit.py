"""Finite source-silhouette study for smooth ring sections, not a geometry pass."""
import hashlib,json,math
from pathlib import Path
import numpy as np
from PIL import Image
from scipy.spatial import ConvexHull
from scipy.optimize import differential_evolution
ROOT=Path(__file__).resolve().parents[3];B=ROOT/'level-editor/work/york-refinement';D=B/'restart7-market-well-study-v1'
def main():
 source=B/'baseline/masks/000049.png';mask=np.array(Image.open(source).convert('L'))>0;yy,xx=np.mgrid[1167:1210,209:253];points=np.column_stack((xx.ravel()+.5,yy.ravel()+.5));target=np.zeros(yy.shape,bool);target[2:41,2:41]=mask;s=math.sin(math.radians(35));c=math.cos(math.radians(35));floor=109.751;angles=np.arange(32)*2*np.pi/32
 def counts(p):
  cx,gy,rx0,ry0,rx1,ry1,z=p;v=np.concatenate([np.column_stack((cx+rx*np.cos(angles),gy-height*c+ry*s*np.sin(angles)))for rx,ry,height in [(rx0,ry0,floor),(rx1,ry1,z)]]);e=ConvexHull(v).equations;inside=np.all(points@e[:,:2].T+e[:,2]<=1e-8,axis=1).reshape(yy.shape);return int((target&~inside).sum()),int((~target&inside).sum())
 def loss(p):
  miss,outside=counts(p);return miss+outside*1.15+.04*((p[2]-p[3])**2+(p[4]-p[5])**2)
 before=[230.8,2245*s,18.8,16.7,19.8,20,129.5];fit=differential_evolution(loss,[(230,233),(1286.7,1289),(16,20),(14,19),(16,20),(17,23),(127,132)],popsize=8,maxiter=55,tol=.005,polish=False,seed=12)
 report=dict(status='Analytic silhouette study only; not saved-model validation',source_mask_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),parameters=['center_x','center_game_y','base_rx','base_ry','top_rx','top_ry','top_scene_z'],current=before,current_miss_outside=counts(before),fit=list(fit.x),fit_miss_outside=counts(fit.x),rounded_for_construction=[231,1288.5,17.5,15.5,18.3,20.6,128],rounded_miss_outside=counts([231,1288.5,17.5,15.5,18.3,20.6,128]),objective=float(fit.fun),limitation='Convex outer silhouette only; excludes physical roof/post occlusion, cavity visibility and bucket. Saved-model physical/native review is required independently.')
 output=D/'ring-silhouette-fit-reproducible.json';assert not output.exists();output.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
if __name__=='__main__':main()
