"""Native first-hit checks along full source crease segments, before Blender."""
import argparse,json,math,hashlib
from pathlib import Path
import numpy as np
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement';S=math.sin(math.radians(35));C=math.cos(math.radians(35))
def check(folder):
 data=json.loads((folder/'geometry.json').read_text());data['53']=json.loads((B/'restart2/bank-continuous-strata-plan-v1/geometry.json').read_text());triangles=[];owners=[]
 for index in ('52','53','54'):
  v=np.array(data[index]['vertices']);triangles.extend(v[np.array(data[index]['faces'])]);owners.extend([index]*len(data[index]['faces']))
 tri=np.array(triangles);projected=np.stack([tri[:,:,0],-tri[:,:,1]*S-tri[:,:,2]*C],axis=2);a=projected[:,0];ab=projected[:,1]-a;ac=projected[:,2]-a;det=ab[:,0]*ac[:,1]-ab[:,1]*ac[:,0];valid=np.abs(det)>1e-10;det[~valid]=1
 results=[]
 for index in ('52','54'):
  vertices=np.array(data[index]['vertices'])
  for line in data[index]['traces']:
   poly=vertices[line['vertices']];samples=[]
   for pa,pb in zip(poly,poly[1:]):
    sa=np.array([pa[0],-pa[1]*S-pa[2]*C]);sb=np.array([pb[0],-pb[1]*S-pb[2]*C]);n=max(2,int(np.linalg.norm(sb-sa)*2)+1)
    samples.extend(pa*(1-t)+pb*t for t in np.linspace(0,1,n))
   for p in samples:
    source=np.array([p[0],-p[1]*S-p[2]*C]);ap=source-a;u=(ap[:,0]*ac[:,1]-ap[:,1]*ac[:,0])/det;w=(ab[:,0]*ap[:,1]-ab[:,1]*ap[:,0])/det;covered=valid&(u>=-1e-8)&(w>=-1e-8)&(u+w<=1+1e-8)
    heights=tri[:,0,2]+u*(tri[:,1,2]-tri[:,0,2])+w*(tri[:,2,2]-tri[:,0,2]);heights[~covered]=-np.inf;nearest=int(np.argmax(heights));delta=float(heights[nearest]-p[2]);results.append(dict(owner=index,trace=line['id'],source=list(source),height_delta_world=delta,firsthit_owner=owners[nearest],blocked=delta>1e-4,miss=not np.any(covered)))
 blocked=[r for r in results if r['blocked']];report=dict(status='HOLD hidden crease samples' if blocked else 'PASS all tested native crease first hits',geometry_sha256=hashlib.sha256((folder/'geometry.json').read_bytes()).hexdigest(),samples=len(results),blocked=len(blocked),misses=sum(r['miss'] for r in results),max_occlusion_world_height=max(r['height_delta_world'] for r in results),blocked_samples=blocked,limits=['CPU triangles before interface Boolean. Saved first-hit and surface-distance checks remain mandatory.','Includes inferred side extensions and interpolated crease segments; no source ownership assignment.','Existing western53 geometry is unchanged.'])
 (folder/'cpu-crease-firsthits.json').write_text(json.dumps(report,indent=2)+'\n');print({k:v for k,v in report.items() if k not in ('blocked_samples','limits')})
if __name__=='__main__':
 parser=argparse.ArgumentParser();parser.add_argument('folder',type=Path);args=parser.parse_args();check(args.folder)
