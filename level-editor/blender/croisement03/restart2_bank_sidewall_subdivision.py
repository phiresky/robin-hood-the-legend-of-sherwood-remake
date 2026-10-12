"""Closed sidewall trial preserving traced front ridges and all old bank vertices."""
import copy,json,hashlib
from pathlib import Path
from collections import Counter
import numpy as np
from scipy.spatial import Delaunay
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/active/croisement03/restart2';O=B/'bank-sidewall-cpu-v1';ray=np.array([0.,-np.cos(np.deg2rad(35)),np.sin(np.deg2rad(35))])
def main():
 O.mkdir(exist_ok=False);source=B/'bank-whole-geometry-plan-v4/geometry.json';data=json.loads(source.read_text());routing=json.loads((B/'bank-shoulder-face-routing-v1/routing.json').read_text());m=data['54'];old=copy.deepcopy(m);vertices=copy.deepcopy(m['vertices']);faces=[];revisions=[];rows=routing['rows']
 for fi,face in enumerate(m['faces']):
  chosen=[(i,r)for i,r in enumerate(rows)if r.get('firsthit_owner')=='54'and r.get('surface')=='side'and r.get('triangle')==fi]
  if not chosen:faces.append(face);continue
  triangle=np.array([vertices[i]for i in face]);normal=np.cross(triangle[1]-triangle[0],triangle[2]-triangle[0]);assert normal@ray>0
  uv=[[0.,0.],[1.,0.],[0.,1.]];points=triangle.tolist();constraints=[];mapping={}
  def add(coord,point):
   for i,p in enumerate(uv):
    if np.linalg.norm(np.array(coord)-p)<1e-8:return i
   uv.append(list(coord));points.append(list(point));return len(uv)-1
  for ri,row in chosen:
   assert min(row['barycentric'])>1e-6
   mapping[ri]=add(row['barycentric'][1:],row['world'])
  for ri,row in chosen:
   if ri+1 in mapping and rows[ri+1]['trace']==row['trace']and mapping[ri]!=mapping[ri+1]:constraints.append((mapping[ri],mapping[ri+1]))
  # Recess a local interior point behind fixed source ridges, never lift anchors.
  for coord in ([.25,.25],[.5,.25],[.25,.5]):
   if min(np.linalg.norm(np.array(coord)-p)for p in uv)>.05:
    point=triangle[0]+coord[0]*(triangle[1]-triangle[0])+coord[1]*(triangle[2]-triangle[0])-ray*1.5;add(coord,point);break
  for iteration in range(16):
   triangulation=Delaunay(np.array(uv));present={tuple(sorted((int(a),int(b))))for f in triangulation.simplices for a,b in zip(f,np.roll(f,-1))};missing=[e for e in constraints if tuple(sorted(e))not in present]
   if not missing:break
   new=[]
   for a,b in constraints:
    if tuple(sorted((a,b)))in present:new.append((a,b));continue
    mid=add((np.array(uv[a])+uv[b])/2,(np.array(points[a])+points[b])/2);assert mid not in (a,b);new.extend([(a,mid),(mid,b)])
   constraints=new
  else:raise AssertionError('Sidewall constraints did not converge')
  local=[*face]+list(range(len(vertices),len(vertices)+len(points)-3));vertices+=points[3:]
  for f in triangulation.simplices:
   a,b,c=map(int,f);t=np.array([points[a],points[b],points[c]])
   if np.cross(t[1]-t[0],t[2]-t[0])@normal<0:b,c=c,b
   faces.append([local[a],local[b],local[c]])
  revisions.append({'old_face':fi,'fixed_source_samples':len(chosen),'new_vertices':len(points)-3,'new_faces':len(triangulation.simplices),'source_constraint_edges':len(constraints)})
 ec=Counter(tuple(sorted((a,b)))for f in faces for a,b in zip(f,f[1:]+f[:1]));assert set(ec.values())=={2};assert len(vertices)-len(ec)+len(faces)==2;assert vertices[:len(old['vertices'])]==old['vertices'];volume=sum(float(np.dot(vertices[f[0]],np.cross(vertices[f[1]],vertices[f[2]])))/6 for f in faces);assert volume>0
 m.update(vertices=vertices,faces=faces,signed_volume=volume,edge_count=len(ec),euler=2,topology='Closed original bank with locally constrained sidewall subdivisions; all original vertices exact.');data['scope']='Private sidewall morphology study, not complete52/54 morphology or material ownership.';data['sidewall_revision']={'source_geometry_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'changed_faces':revisions,'all_old_vertices_exact':True,'fixed_crest_preserved':True,'ray_recess':1.5};(O/'geometry.json').write_text(json.dumps(data,indent=2)+'\n');print({'revised_faces':len(revisions),'vertices':len(vertices),'closed_euler':2,'all_old_vertices_exact':True})
if __name__=='__main__':main()
