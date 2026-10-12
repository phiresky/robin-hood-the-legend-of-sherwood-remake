"""Round and overlap wing-region borders while preserving the total rest silhouette."""
import argparse,json,hashlib
from pathlib import Path
import numpy as np
from matplotlib.path import Path as Polygon

def main():
 p=argparse.ArgumentParser();p.add_argument('--candidate',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();a.output.mkdir(exist_ok=False);fit=json.loads((a.candidate/'fit.json').read_text());spec=fit['fixed_geometry'];regions=[];scale=fit['fixed_uniform_scale']
 for index,old in enumerate(spec['wing_regions']):
  old=np.asarray(old);new=[];found=0
  for left,right in zip(old,np.roll(old,-1,axis=0)):
   new.append(left)
   if abs(left[1])<1e-8 and abs(right[1])<1e-8:
    found+=1;control=(left+right)/2;control[1]=(1 if index==0 else -1)*1.5*scale
    for t in np.linspace(0,1,9)[1:-1]:new.append((1-t)**2*left+2*t*(1-t)*control+t*t*right)
  assert found==1;regions.append(np.asarray(new))
 xy=np.array([[x,y] for x in np.linspace(-.1,8.1,301)*scale for y in np.linspace(-7.1,6.1,401)*scale]);original=Polygon(np.asarray(spec['wing_outline'])[:,:2]).contains_points(xy);new=np.logical_or.reduce([Polygon(x[:,:2]).contains_points(xy) for x in regions]);assert np.array_equal(original,new),'Rounded partition changed total silhouette'
 spec['wing_regions']=[x.tolist() for x in regions];spec['articulation']='Separate fore/hind wings with rounded overlapping internal borders. Total rest silhouette unchanged; border shape inferred.'
 oldtext=(a.candidate/'geometry.py').read_text();begin=oldtext.index('SPEC=');end=oldtext.index('\nWING=',begin);helper=a.output/'geometry.py';helper.write_text(oldtext[:begin]+'SPEC='+repr(spec)+oldtext[end:]);fit['geometry_helper_sha256']=hashlib.sha256(helper.read_bytes()).hexdigest();fit['status']='PRIVATE_ROUNDED_BORDER_SEED_METRICS_STALE_UNTIL_REFIT';fit['rounded_border_parent_sha256']=hashlib.sha256((a.candidate/'fit.json').read_bytes()).hexdigest();(a.output/'fit.json').write_text(json.dumps(fit,indent=2)+'\n');(a.output/'partition-check.json').write_text(json.dumps({'status':'PASS_REST_UNION_ONLY','sample_points':len(xy),'limits':['Internal border shape is an inferred anatomy hypothesis. Articulated poses, texture ownership and contact must be rechecked.']},indent=2)+'\n')
if __name__=='__main__':main()
