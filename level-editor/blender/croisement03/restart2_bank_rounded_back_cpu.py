"""Rebuild inferred enclosing shoulders with rounded cross-sections behind traced fronts."""
import json,math,hashlib
from pathlib import Path
from collections import Counter
import numpy as np
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/active/croisement03/restart2';O=B/'bank-rounded-backing-cpu-v1';E=B/'bank-rounded-east-cpu-v1';S=math.sin(math.radians(35));C=math.cos(math.radians(35))
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def rebuild(mesh,template,owner):
 old=np.array(mesh['vertices']);faces=mesh['faces'];nr=len(template['ring_names']);n=len(template['vertices'])//nr;nf=n-3;assert n*nr==len(template['vertices']);front=set(i for f in faces[-mesh['front_reconstruction']['new_front_triangles']:]for i in f);v=old.tolist();params={};fixed=set(front)
 for i in range(n*nr):
  row,col=divmod(i,n)
  if col in (0,nf-1):params[i]=(float(row),math.pi if col==0 else 0.)
  elif col>=nf:params[i]=(float(row),(col-nf+1)*math.pi/4)
  if row in (0,nr-1):fixed.add(i)
 mid={};newfaces=[]
 for f in faces:
  mids=[]
  for a,b in zip(f,f[1:]+f[:1]):
   key=tuple(sorted((a,b)))
   if key not in mid:
    k=len(v);mid[key]=k;v.append(((old[a]+old[b])/2).tolist())
    if a in fixed and b in fixed:fixed.add(k)
    if a in params and b in params:params[k]=tuple((np.array(params[a])+params[b])/2)
   mids.append(mid[key])
  a,b,c=f;ab,bc,ca=mids;newfaces.extend([[a,ab,ca],[ab,b,bc],[ca,bc,c],[ab,bc,ca]])
 before=np.array(v);verts=before.copy();changes=[]
 for i,(row,theta)in params.items():
  if i in fixed:continue
  r0=int(math.floor(row));r1=min(r0+1,nr-1);t=row-r0
  left=old[r0*n]*(1-t)+old[r1*n]*t;right=old[r0*n+nf-1]*(1-t)+old[r1*n+nf-1]*t;u=(1+math.cos(theta))/2;center=(left+right)/2;width=right[0]-left[0];assert width>0
  # The back is a horizontal enclosing mass, not a displaced screen-plane strip.
  # Wide convex sections narrow continuously at the source-indicated joints.
  sy=-center[1]*S-center[2]*C
  lobes=[(364,12),(382,13),(401,14),(435,15)]if owner==54 else[(202,14),(224,15),(246,14),(276,20)]
  shape=max(math.exp(-((sy-y)/radius)**2)for y,radius in lobes)
  depth=width*(.42+.38*shape)
  target=left*(1-u)+right*u;target[0]=center[0]+width*.5*math.cos(theta);target[1]+=depth*math.sin(theta)
  # Preserve exact top and ground boundaries; blend adjoining sections into them.
  blend=min(1.,row/3.,(nr-1-row)/2.);blend=blend*blend*(3-2*blend)
  target=before[i]*(1-blend)+target*blend;assert target[2]>=-1e-8
  verts[i]=target;changes.append({'vertex':i,'old':before[i].tolist(),'new':target.tolist(),'ring':row,'angle':theta})
 assert np.array_equal(verts[list(fixed)],before[list(fixed)]);ec=Counter(tuple(sorted((a,b)))for f in newfaces for a,b in zip(f,f[1:]+f[:1]));assert set(ec.values())=={2};assert len(verts)-len(ec)+len(newfaces)==2
 volume=sum(float(np.dot(verts[f[0]],np.cross(verts[f[1]],verts[f[2]])))/6 for f in newfaces);assert volume>0
 result=dict(mesh);result.update(vertices=verts.tolist(),faces=newfaces,signed_volume=volume,rounded_backing={'source_front_original_vertices_exact':len(front),'fixed_vertices':len(fixed),'changes':changes,'old_depth_authority':'Previous three-point hidden backing is an inference, superseded privately.','construction':'Semicircular enclosing horizontal shoulder cross-sections, with irregular source-indicated mass widths, exact source-front mesh and top/ground contacts.','limits':['Hidden shape is inferred. New silhouette/ownership and saved Boolean contacts must pass independently.','No source assignment or visual approval.']});return result

def main():
 assert not O.exists()and not E.exists();O.mkdir();E.mkdir();p=B/'bank-boulder-fronts-cpu-v8/geometry.json';ep=B/'bank-boulder-east-cpu-v8/geometry.json';d=json.loads(p.read_text());e=json.loads(ep.read_text());template=json.loads((B/'bank-shoulder-lofts-cpu-v7/geometry.json').read_text())['54'];et=json.loads((B/'bank-east-loft-cpu-v8/geometry.json').read_text());d['54']=rebuild(d['54'],template,54);e=rebuild(e,et,52);d['rounded_backing_inputs']={str(p):sha(p),str(ep):sha(ep)};(O/'geometry.json').write_text(json.dumps(d,separators=(',',':'))+'\n');(E/'geometry.json').write_text(json.dumps(e,separators=(',',':'))+'\n');print({name:{'vertices':len(m['vertices']),'faces':len(m['faces']),'changed':len(m['rounded_backing']['changes'])}for name,m in [('south',d['54']),('east',e)]})
if __name__=='__main__':main()
