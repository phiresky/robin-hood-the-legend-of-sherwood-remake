"""Locate source silhouette residuals and abrupt transitions for a private anatomy fit."""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import argparse,json,importlib.util
from pathlib import Path
import numpy as np
from PIL import Image
from scipy.spatial import ConvexHull
from scipy.spatial.transform import Rotation
from matplotlib.path import Path as Polygon
from hashlib import sha256

def main():
 p=argparse.ArgumentParser();p.add_argument('--candidate',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();a.output.mkdir(exist_ok=False)
 fit=json.loads((a.candidate/'fit.json').read_text());spec=importlib.util.spec_from_file_location('candidate',a.candidate/'geometry.py');geo=importlib.util.module_from_spec(spec);spec.loader.exec_module(geo);rows=[];edges=[]
 for row in fit['rows']:
  path=Path(row['source']['source']);assert sha256(path.read_bytes()).hexdigest()==row['source']['sha256'];im=np.array(Image.open(path).convert('RGBA'));y,x=np.nonzero(im[:,:,3]);points=np.c_[x+.5,y+.5];shift=points.mean(0)+row['parameters'][5:7];body,wings=geo.geometry(row['parameters']);polys=[body[:,:2][ConvexHull(body[:,:2]).vertices]+shift,*[w[:,:2]+shift for w in wings]]
  pred=np.logical_or.reduce([Polygon(poly).contains_points(points) for poly in polys]);missing=points[~pred];dist=np.full(len(missing),np.inf)
  for poly in polys:
   for v,w in zip(poly,np.roll(poly,-1,axis=0)):
    d=w-v;t=np.clip((missing-v)@d/max(d@d,1e-12),0,1);dist=np.minimum(dist,np.linalg.norm(missing-(v+t[:,None]*d),axis=1))
  pixels=[dict(x=float(q[0]),y=float(q[1]),distance=float(d),rgb=im[int(q[1]),int(q[0]),:3].tolist()) for q,d in zip(missing,dist)]
  assert len(pixels)==row['missing'],(row['phase'],len(pixels),row['missing'])
  rows.append(dict(phase=row['phase'],missing=pixels,within_half_pixel=int((dist<=.5).sum()),half_to_one_pixel=int(((dist>.5)&(dist<=1)).sum()),beyond_one_pixel=int((dist>1).sum())))
  nxt=fit['rows'][(row['phase']+1)%99];r0=Rotation.from_quat(row['quaternion']);r1=Rotation.from_quat(nxt['quaternion']);edges.append(dict(phase=row['phase'],body_turn_degrees=float(np.degrees((r0.inv()*r1).magnitude())),hinge_turn_degrees=(np.array(nxt['parameters'][3:5])-row['parameters'][3:5]).tolist()))
 report=dict(status='PRIVATE_RESIDUAL_LOCALIZATION_NOT_ACCEPTANCE',candidate_sha256=sha256((a.candidate/'fit.json').read_bytes()).hexdigest(),geometry_sha256=sha256((a.candidate/'geometry.py').read_bytes()).hexdigest(),recipe_sha256=sha256(Path(__file__).read_bytes()).hexdigest(),totals={k:sum(r[k] for r in rows) for k in ['within_half_pixel','half_to_one_pixel','beyond_one_pixel']},worst_turns=sorted(edges,key=lambda x:x['body_turn_degrees'],reverse=True)[:12],rows=rows,limits=['Distances measured in source-image pixels from missed pixel centers to projected physical surfaces.','Subpixel residuals are not waived; source binary artwork and rendered raster still need visual comparison.','Turns are source-phase body/hinge changes, not contact/self-intersection proof.'])
 (a.output/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:report[k] for k in ['totals','worst_turns']}))
if __name__=='__main__':main()
