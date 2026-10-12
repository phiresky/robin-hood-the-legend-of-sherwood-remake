"""Strict nonadjacent triangle crossing diagnostic for private bank53 CPU arrays."""
import argparse,hashlib,json
from pathlib import Path
import numpy as np

def main():
 p=argparse.ArgumentParser();p.add_argument('--geometry',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();data=json.loads(a.geometry.read_text());v=np.array(data['vertices']);f=np.array(data['faces']);t=v[f];lo=t.min(1);hi=t.max(1);pairs=[];candidates=0;epsilon=1e-7
 def segment_tri(orig,direction,tri):
  e1=tri[:,1]-tri[:,0];e2=tri[:,2]-tri[:,0];p=np.cross(direction,e2);det=(e1*p).sum(1);valid=np.abs(det)>1e-9;inv=np.divide(1.,det,out=np.zeros_like(det),where=valid);s=orig-tri[:,0];u=(s*p).sum(1)*inv;q=np.cross(s,e1);w=(direction*q).sum(1)*inv;distance=(e2*q).sum(1)*inv;return valid&(u>epsilon)&(w>epsilon)&(u+w<1-epsilon)&(distance>epsilon)&(distance<1-epsilon)
 for i in range(len(f)):
  ids=np.flatnonzero(np.all(hi>=lo[i]-1e-8,axis=1)&np.all(lo<=hi[i]+1e-8,axis=1)&(np.arange(len(f))>i));ids=ids[~np.any(f[ids,:,None]==f[i][None,None,:],axis=(1,2))];candidates+=len(ids)
  if not len(ids):continue
  others=t[ids];hit=np.zeros(len(ids),bool)
  for edge in range(3):
   hit|=segment_tri(t[i,edge],t[i,(edge+1)%3]-t[i,edge],others)
   hit|=segment_tri(others[:,edge],others[:,(edge+1)%3]-others[:,edge],np.broadcast_to(t[i],others.shape))
  pairs.extend([[i,int(j)]for j in ids[hit]])
 a.output.mkdir(exist_ok=False);r={'status':'PASS_NO_STRICT_NONADJACENT_CROSSINGS'if not pairs else'HOLD_STRICT_INTERNAL_CROSSINGS','geometry_sha256':hashlib.sha256(a.geometry.read_bytes()).hexdigest(),'triangles':len(f),'aabb_nonadjacent_pairs_tested':candidates,'crossing_pairs':pairs,'crossing_pair_count':len(pairs),'limits':['Tests proper noncoplanar segment/triangle crossings with strict interior epsilon1e-7.','Shared-vertex pairs, tangencies and coplanar overlap are excluded; this is not a complete watertight solid certification.']};(a.output/'receipt.json').write_text(json.dumps(r,indent=2)+'\n');print({k:r[k]for k in ['status','triangles','aabb_nonadjacent_pairs_tested','crossing_pair_count']})
if __name__=='__main__':main()
