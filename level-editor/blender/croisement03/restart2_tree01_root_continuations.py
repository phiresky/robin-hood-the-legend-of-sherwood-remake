"""Infer closed root collars below the last observed bark without altering it."""
import json,math,hashlib
from pathlib import Path
from collections import Counter
import numpy as np
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement/restart2';O=B/'tree01-cpu-roots-v1'
def main():
 inp=B/'tree01-cpu-wood-v3/geometry.json';data=json.loads(inp.read_text());O.mkdir(exist_ok=False);rows=[]
 for p in data['parts']:
  n=p['ring_count'];r=p['vertices_per_ring'];old=np.array(p['vertices']);rings=[old[i*r:(i+1)*r].copy()for i in range(n)];center=rings[-1].mean(0);prev=rings[-2].mean(0);width=float(np.ptp(rings[-1][:,0]));assert center[2]>0
  # Continue the lower tangent, easing its sideways component toward vertical.
  # This is an explicit hidden-ground hypothesis, never source-visible root art.
  tangent=center-prev;tangent=tangent/(-tangent[2]);extra=[]
  for i in range(1,9):
   t=i/8;z=center[2]*(1-t);travel=center[2]*t;drift=tangent[:2]*travel*(1-.65*t);c=np.array([center[0]+drift[0],center[1]+drift[1],z]);flare=1+.6*t**4;w=width*flare;ring=[(c+np.array([math.cos(j*2*math.pi/r)*w/2,math.sin(j*2*math.pi/r)*w/2,0])).tolist()for j in range(r)];extra.append(ring)
  rings+=extra;vertices=np.array(rings).reshape((-1,3)).tolist();faces=[]
  for row in range(len(rings)-1):
   for i in range(r):
    a=row*r+i;b=row*r+(i+1)%r;c=b+r;dd=a+r;faces.extend([[a,b,c],[a,c,dd]])
  for row,reverse in [(0,True),(len(rings)-1,False)]:
   ci=len(vertices);vertices.append(np.mean(rings[row],axis=0).tolist())
   for i in range(r):
    f=[ci,row*r+i,row*r+(i+1)%r];faces.append(f[::-1]if reverse else f)
  volume=sum(float(np.dot(vertices[f[0]],np.cross(vertices[f[1]],vertices[f[2]])))/6 for f in faces)
  if volume<0:faces=[f[::-1]for f in faces];volume=-volume
  edges=Counter(tuple(sorted((a,b)))for f in faces for a,b in zip(f,f[1:]+f[:1]));assert set(edges.values())=={2}and len(vertices)-len(edges)+len(faces)==2 and volume>0
  assert np.array_equal(np.array(vertices[:n*r]),old[:n*r]);assert all(abs(np.ptp(np.array(q)[:,0])-np.ptp(np.array(q)[:,1]))<1e-8 for q in rings)
  p.update(vertices=vertices,faces=faces,volume=volume,ring_count=len(rings),hidden_extension_sections=p['hidden_extension_sections']+list(range(n,len(rings))),root_ground_hypothesis_z=0.,root_flare_ratio=1.6)
  rows.append({'context':p['source_obstacle_context'],'retained_rings':n,'added_rings':8,'width_equals_depth':True,'bottom_z':0.,'inferred':True})
 data['root_revision']={'input':str(inp),'sha256':hashlib.sha256(inp.read_bytes()).hexdigest(),'parts':rows,'limits':['Ground plane contact is a hidden inference; native bank first-hit guards and saved context review required.','No bark source ownership, root pixels, foliage, or geometry approval added.']};(O/'geometry.json').write_text(json.dumps(data,indent=2)+'\n');print(rows)
if __name__=='__main__':main()
