"""Fit circular wood sections inside source-owned solid body runs, excluding fibers."""
import argparse,hashlib,json
from pathlib import Path
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/active/croisement01/mission-net01-oct12-v1'
def main():
 ap=argparse.ArgumentParser();ap.add_argument('--continuous',action='store_true');args=ap.parse_args()
 sources=json.loads((OUT/'source.json').read_text())['sources'];result={}
 for key,y0,y1 in [('initial',3,36),('e',87,113),('i',87,113)]:
  src=sources[key];assert hashlib.sha256(Path(src['path']).read_bytes()).hexdigest()==src['sha256'];alpha=np.asarray(Image.open(src['path']).convert('RGBA'))[:,:,3]>0;profile=[]
  for x in range(alpha.shape[1]):
   ys=np.where(alpha[y0:y1,x])[0]+y0
   if not len(ys):continue
   runs=np.split(ys,np.where(np.diff(ys)>1)[0]+1);run=max(runs,key=len)
   if len(run)<6:continue
   profile.append([x+.5,float((run[0]+run[-1]+1)/2),float(len(run)/2-.2)])
  assert len(profile)>10
  assert all(abs(b[0]-a[0]-1)<1e-8 for a,b in zip(profile,profile[1:]))
  qualification={}
  if args.continuous:
   rows=np.asarray(profile);x=rows[:,0];body=rows[:,2]>=np.median(rows[:,2])*.8
   # A single smooth axis avoids interpreting fibers and alpha notches as log bends.
   coeff=np.polyfit(x[body],rows[body,1],1);axis=np.polyval(coeff,x);clearance=rows[:,2]-np.abs(axis-rows[:,1]);threshold=np.median(rows[body,2])*.55
   good=np.where(clearance>=threshold)[0];runs=np.split(good,np.where(np.diff(good)>1)[0]+1);run=max(runs,key=len);assert len(run)>10
   left=x[run[0]];right=x[run[-1]];cap=min(6.,(right-left)/5)
   # Quarter-ellipse end caps have continuous joins to the cylindrical body.
   def shape(t):
    edge=min(t-left,right-t)
    return np.sqrt(max(0.,1-(max(0.,cap-edge)/cap)**2))
   radius=min(float(np.median(rows[body,2])),min(float(clearance[i]/shape(x[i])) for i in run if shape(x[i])>.01))-.15
   assert radius>2
   samples=np.linspace(left,right,int((right-left)*4)+1)
   profile=[[float(t),float(np.polyval(coeff,t)),max(.1,float(radius*shape(t)))] for t in samples]
   for i in run:
    assert radius*shape(x[i])<=clearance[i]+1e-8
   qualification={'axis_linear_coefficients':coeff.tolist(),'body_radius':radius,'source_span':[left,right],'cap_length':cap,'native_column_clearance_min':float(min(clearance[run]-np.array([radius*shape(t) for t in x[run]]))),'scope':'Continuous central body lies inside source solid runs at every native column; irregular silhouette remains separate thin shell. End caps are inferred.'}
  else:
   profile=[[profile[0][0]-.45,profile[0][1],.1],*profile,[profile[-1][0]+.45,profile[-1][1],.1]]
  result[key]={'source_sha256':src['sha256'],'profile':profile,'continuous_fit':qualification,'cross_section':'Circular in world YZ; screen vertical radius equals world radius at35degree elevation.','scope':'Native solid body runs define core; observed alpha fibers remain a separate thin shell.'}
 (OUT/('wood-cores-continuous-v2.json' if args.continuous else 'wood-cores-v1.json')).write_text(json.dumps(result,indent=2)+'\n');print({k:len(v['profile'])for k,v in result.items()})
if __name__=='__main__':main()
