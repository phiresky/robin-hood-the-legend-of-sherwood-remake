"""Check approved gate translations against the inferred room solids without rendering."""
import json,hashlib
from pathlib import Path
import numpy as np
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/active/york/restart2';OUT=W/'restart61-room-gate-contact-v13';assert not OUT.exists()
source=W/'gate-saved-contact-audit-v1/report.json';audit=json.loads(source.read_text());assert audit['model_sha256']=='6ed0064f493f79cb38f249fdbc41177eac3cee5659fb86c73f59ae746ad19b70';motion=W/'jamb-clearance-candidate-v1/motion-proposal.json';assert hashlib.sha256(motion.read_bytes()).hexdigest()=='07fd102ae0a508bf77ff10f6d52dda6c746723d3da7011b312b9164b7758a446';room=json.loads((W/'restart61-room-winch-contact-v13/report.json').read_text());gv=np.array(audit['geometry_world']['gate_vertices']);gf=audit['geometry_world']['gate_faces'];gids=[(f[0],f[i],f[i+1])for f in gf for i in range(1,len(f)-1)]
def segment_triangle(p,q,tri):
 e1=tri[:,1]-tri[:,0];e2=tri[:,2]-tri[:,0];direction=q-p;h=np.cross(direction,e2);det=np.einsum('ij,ij->i',e1,h);valid=np.abs(det)>1e-9;inv=np.zeros_like(det);inv[valid]=1/det[valid];ss=p-tri[:,0];u=np.einsum('ij,ij->i',ss,h)*inv;r=np.cross(ss,e1);v=r@direction*inv;t=np.einsum('ij,ij->i',e2,r)*inv;return np.where(valid&(u>=-1e-7)&(v>=-1e-7)&(u+v<=1+1e-7)&(t>1e-6)&(t<1-1e-6))[0]
parts=[]
for name,box in room['inferred_receiver_bounds'].items():
 lo=np.array(box)[:,0];hi=np.array(box)[:,1];vv=np.array([[x,y,z]for x in [lo[0],hi[0]]for y in [lo[1],hi[1]]for z in [lo[2],hi[2]]]);quads=[(0,1,3,2),(4,6,7,5),(0,4,5,1),(2,3,7,6),(0,2,6,4),(1,5,7,3)];tr=vv[[[f[0],f[i],f[i+1]]for f in quads for i in [1,2]]];parts.append((name,lo,hi,tr))
rows=[]
for pose in json.loads(motion.read_text())['rows']:
 gate=gv+np.array([0,0,pose['nominal_lift_world_z']]);gt=gate[gids];hits=[]
 for name,lo,hi,tr in parts:
  if not(np.all(gate.max(axis=0)>=lo)and np.all(gate.min(axis=0)<=hi)):continue
  pairs=0
  for t in gt:
   if not(np.all(t.max(axis=0)>=lo)and np.all(t.min(axis=0)<=hi)):continue
   found=any(len(segment_triangle(t[i],t[j],tr))for i,j in [(0,1),(1,2),(2,0)])
   if not found:found=any(len(segment_triangle(tt[i],tt[j],t[None]))for tt in tr for i,j in [(0,1),(1,2),(2,0)])
   pairs+=int(found)
  inside=int(np.count_nonzero(np.all(gate>lo+.005,axis=1)&np.all(gate<hi-.005,axis=1)))
  if pairs or inside:hits.append({'receiver':name,'crossing_gate_triangles':pairs,'gate_vertices_strictly_inside':inside})
 rows.append({'frame':pose['frame'],'hits':hits})
report={'status':'PASS_NO_CONFIRMED_GATE_CROSSINGS'if not any(r['hits']for r in rows)else'HOLD_GATE_CROSSINGS','model_sha256':room['model_sha256'],'gate_geometry_audit_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'motion_sha256':hashlib.sha256(motion.read_bytes()).hexdigest(),'rows':rows,'limitations':'Strict segment/triangle crossing and gate-vertex containment tests. Coplanar contact and room-box containment wholly within a gate component are not certified.'};OUT.mkdir();(OUT/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
