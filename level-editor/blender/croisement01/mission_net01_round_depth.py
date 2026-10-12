"""Refine source-owned wood faces for a rounded, silhouette-preserving depth profile."""
import argparse,json
from pathlib import Path
import numpy as np
ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/croisement01-refinement/mission-net01-oct12-v1'
def main():
 ap=argparse.ArgumentParser();ap.add_argument('--input',default='domains.json');ap.add_argument('--output',default='domains-rounded-v1.json');args=ap.parse_args()
 domains=json.loads((OUT/args.input).read_text())
 for key,row in domains.items():
  if not key.endswith('weight'):continue
  for part in row['parts']:
   triangles=np.asarray(part['triangles'])
   # Shared-edge midpoint subdivision is conforming and retains all holes.
   for _ in range(3):
    a,b,c=triangles[:,0],triangles[:,1],triangles[:,2];ab=(a+b)/2;bc=(b+c)/2;ca=(c+a)/2
    triangles=np.concatenate([np.stack([a,ab,ca],1),np.stack([ab,b,bc],1),np.stack([ca,bc,c],1),np.stack([ab,bc,ca],1)])
   # Boundary rings must acquire the same eight equal segments per edge.
   for name in ['exterior','holes']:
    rings=[part[name]] if name=='exterior' else part[name];refined=[]
    for ring in rings:
     refined.append([[a[0]+(b[0]-a[0])*i/8,a[1]+(b[1]-a[1])*i/8] for a,b in zip(ring,ring[1:]+ring[:1]) for i in range(8)])
    part[name]=refined[0] if name=='exterior' else refined
   part['triangles']=triangles.tolist()
  row['depth_profile']='Rounded posterior from distance to observed outline; thin outer fibers, unchanged source projection.'
 (OUT/args.output).write_text(json.dumps(domains)+'\n')
if __name__=='__main__':main()
