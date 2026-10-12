"""Infer rounded rock bulk behind fixed source-facing boulders and measured contacts."""
import json,math,hashlib
from pathlib import Path
from collections import Counter
import numpy as np
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement/restart2';O=B/'bank-wrapped-masses-cpu-v1';E=B/'bank-wrapped-east-cpu-v1';S=math.sin(math.radians(35));C=math.cos(math.radians(35));RAY=np.array([0.,-C,S])
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def project(v):return np.column_stack((v[:,0],-v[:,1]*S-v[:,2]*C))
def revise(mesh,owner,proposals):
 d=dict(mesh);vertices=[list(v)for v in mesh['vertices']];faces=[list(f)for f in mesh['faces']];old=np.array(vertices);front_start=len(faces)-mesh['front_reconstruction']['new_front_triangles'];front=set(i for f in faces[front_start:]for i in f);top=max(v[2]for v in vertices);fixed=set(front)|{i for i,v in enumerate(vertices)if v[2]<1e-6 or v[2]>=100 or(owner==52 and v[0]<=560+1e-6)};edge_mid={};newfaces=[];front_new=set(front)
 for face_index,f in enumerate(faces):
  mids=[]
  for a,b in zip(f,f[1:]+f[:1]):
   key=tuple(sorted((a,b)))
   if key not in edge_mid:
    edge_mid[key]=len(vertices);vertices.append(((old[a]+old[b])/2).tolist())
    if a in fixed and b in fixed:fixed.add(edge_mid[key])
   mid=edge_mid[key];mids.append(mid)
   if face_index>=front_start:fixed.add(mid);front_new.add(mid)
  a,b,c=f;ab,bc,ca=mids;newfaces.extend([[a,ab,ca],[ab,b,bc],[ca,bc,c],[ab,bc,ca]])
 v=np.array(vertices);before=v.copy();native=project(v);changes=[]
 for i,p in enumerate(v):
  if i in fixed:continue
  xy=native[i];amplitude=0.
  for face in proposals:
   poly=np.array(face['native_outline']);center=(poly.min(0)+poly.max(0))/2;span=np.ptp(poly,axis=0);radius=np.maximum(span*.65,4);rho=float(np.sum(((xy-center)/radius)**2));weight=math.exp(-rho*1.8);amplitude=max(amplitude,min(16.,float(min(span))*.85)*weight)
  # A complete convex back follows the same visible mass locations. The
  # recession is depth inference and cannot establish source-pixel ownership.
  amount=amplitude*math.sin(math.pi*max(0,min(1,p[2]/top)))**.6;amount=min(amount,max(0,p[2]-.01)/S);v[i]-=RAY*amount
  if amount>1e-8:changes.append({'vertex':i,'recession_world':amount,'old':before[i].tolist(),'new':v[i].tolist()})
 assert np.max(np.abs(project(v)-native))<1e-10;assert np.max(np.abs(v[list(fixed)]-before[list(fixed)]))==0
 counts=Counter(tuple(sorted((a,b)))for f in newfaces for a,b in zip(f,f[1:]+f[:1]));assert set(counts.values())=={2};assert len(v)-len(counts)+len(newfaces)==2
 volume=sum(float(np.dot(v[f[0]],np.cross(v[f[1]],v[f[2]])))/6 for f in newfaces);assert volume>0
 d.update(vertices=v.tolist(),faces=newfaces,signed_volume=volume,wrapped_mass_inference={'previous_mesh_sha256':hashlib.sha256(json.dumps(mesh,sort_keys=True).encode()).hexdigest(),'source_front_vertices_locked':len(front_new),'total_locked':len(fixed),'native_projection_error':float(np.max(np.abs(project(v)-native))),'changes':changes,'max_recession_world':max([r['recession_world']for r in changes],default=0),'scope':'First private hidden-volume hypothesis following complete stone masses. Front projection, all source-facing triangles, crest and grounded boundary preserved.','limits':['Depth and side/back shape remain inferred; saved solid intersections and close oblique review required.','Not a visual acceptance or source ownership receipt.']});return d

def main():
 assert not O.exists()and not E.exists();O.mkdir();E.mkdir();source=B/'bank-boulder-fronts-cpu-v8/geometry.json';east=B/'bank-boulder-east-cpu-v8/geometry.json';d=json.loads(source.read_text());e=json.loads(east.read_text());proposal=json.loads((B/'bank-visible-face-traces-v1/faces.json').read_text());d['54']=revise(d['54'],54,[f for f in proposal['faces']if f['owner']==54]);e=revise(e,52,[f for f in proposal['faces']if f['owner']==52]);d['wrapped_mass_inputs']={str(source):sha(source),str(east):sha(east)};(O/'geometry.json').write_text(json.dumps(d,separators=(',',':'))+'\n');(E/'geometry.json').write_text(json.dumps(e,separators=(',',':'))+'\n');print({name:{'vertices':len(m['vertices']),'faces':len(m['faces']),'max_recession':m['wrapped_mass_inference']['max_recession_world']}for name,m in [('south',d['54']),('east',e)]})
if __name__=='__main__':main()
