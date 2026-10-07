"""Plan exact local unions only at retained rooted attachment hypotheses."""
import hashlib,json
from pathlib import Path
import numpy as np
R=Path(__file__).resolve().parents[3]/'level-editor/work/croisement01-refinement/restart2';p=R/'tree08-v12-chain-cpu-v3';r=json.loads((p/'report.json').read_text());data=np.load(p/'mesh.npz');h=json.loads((R/'tree08-wood-prototype-v10/rooted-depth-hierarchy.json').read_text());arcs={q['trace_id']:q for q in h['arcs']};s,c=np.sin(np.radians(35)),np.cos(np.radians(35));eligible={q['index']:q for q in r['mesh_sections'] if not q['held_crossing']};points={};centers={};ends={}
for i,section in eligible.items():
 points[i]={tuple(point[:2]) for trace_id in section['source_traces'] for point in arcs[trace_id]['source_points']}
 centers[i]=data[f'vertices_{i}'].reshape(-1,16,3).mean(1)
 ends[i]=[(end,tuple(np.round([centers[i][end,0],-centers[i][end,1]*s-centers[i][end,2]*c],6))) for end in [0,-1]]
edges=[];adj={i:set() for i in eligible}
for i in eligible:
 for end,native in ends[i]:
  for j in eligible:
   if i==j or native not in points[j]:continue
   distance=float(np.min(np.linalg.norm(centers[j]-centers[i][end],axis=1)))
   # Gaussian center smoothing can move an interior sample subpixel. A large
   # world-space gap is a depth/crossing issue and must not be unioned blindly.
   if distance>2:continue
   adj[i].add(j);adj[j].add(i);edges.append(dict(a=i,b=j,a_endpoint=end,native=list(native),world_center_distance=distance))
visited=set();groups=[]
for i in eligible:
 if i in visited:continue
 stack=[i];group=[];visited.add(i)
 while stack:
  node=stack.pop();group.append(node)
  for other in sorted(adj[node]):
   if other not in visited:visited.add(other);stack.append(other)
 groups.append(sorted(group))
result=dict(status='CPU planning only; exact union still requires volume, manifold and internal-face validation',mesh_sha256=r['mesh_sha256'],rooted_attachment_hypotheses=edges,groups=sorted(groups,key=len,reverse=True),held_sections=[q['index'] for q in r['mesh_sections'] if q['held_crossing']],union_basis='Matching original rooted source points and inherited physical depth; ambiguous crossing sections excluded. Anatomical inference remains subject to actual-view review.',no_global_remesh=True)
with (p/'fork-union-plan.json').open('x') as f:json.dump(result,f,indent=2);f.write('\n')
print('Eligible',len(eligible),'held',len(result['held_sections']),'groups',list(map(len,result['groups'])),'edges',len(edges))
