"""Rebuild the eastern bank as connected rock courses around exact native strata."""
import json,math,hashlib
from pathlib import Path
from collections import Counter
import numpy as np
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/active/croisement03/restart2';O=B/'bank-east-loft-cpu-v8';S=math.sin(math.radians(35));C=math.cos(math.radians(35))
def main():
 O.mkdir(exist_ok=False);inp=B/'bank-sidewall-cpu-v1/geometry.json';data=json.loads(inp.read_text());mesh=data['52'];traces={t['id']:[mesh['vertices'][i]for i in t['vertices']]for t in mesh['traces']};xs=[560,575,587,590,594,596,603,606,610,613,616];rings=[];names=[]
 def ring(height,front,name):
  # Front points follow the visible rock course; the rear closes into bank bulk.
  # A planar joining strip at x560..565 is hidden inside the retained main bank.
  ring=[[x,-float(np.interp(x,[p[0]for p in front],[p[1]for p in front]))/S,height/C]for x in xs]
  ring += [[610,-max(270,height+186)/S,height/C],[575,-max(250,height+186)/S,height/C],[560,-max(245,height+186)/S,height/C]];rings.append(ring);names.append(name)
 ring(85,[(560,292),(575,281),(590,274),(603,276),(610,281),(613,285),(616,286)],'upper inferred shoulder')
 left={'east-upper-block-upper':300.,'east-upper-block-lower':301.5,'east-middle-block-upper':330.,'east-middle-block-lower':331.5,'east-low-block-upper':340.,'east-low-block-lower':341.5}
 for name in left:
  points=traces[name];height={'east-upper-block-upper':75.,'east-upper-block-lower':67.5,'east-middle-block-upper':54.,'east-middle-block-lower':43.5,'east-low-block-upper':34.,'east-low-block-lower':21.5}[name];native=[[p[0],-p[1]*S-p[2]*C+height]for p in points];front=[[560,left[name]],[575,(left[name]+native[0][1])/2],[587,native[0][1]-7],[590,native[0][1]-15]]+native+[[616,native[-1][1]-2]];ring(height,front,name)
 ring(0,[(560,350),(575,345),(590,334),(603,323),(610,320),(616,317)],'inferred lower closure')
 expanded=[];expanded_names=[]
 for i,ring in enumerate(rings):
  expanded.append(ring);expanded_names.append(names[i])
  if i<len(rings)-1:
   a=np.array(ring);b=np.array(rings[i+1]);gap=float(np.min(a[:,2]-b[:,2]));assert gap>0;belly=i in (0,1,3,5,6)
   for t in (.18,.38,.62,.82):
    mid=a*(1-t)+b*t;phase=np.array(xs);lobes=.35+.65*np.maximum(np.exp(-((phase-(601+i%2*3))/9)**2),.75*np.exp(-((phase-579)/8)**2));lobes[:1]=0
    weights=np.r_[lobes,[0.,0.,0.]];distance=min(13. if belly else 8.,gap*.40/S)*math.sin(math.pi*t)*(1 if belly else -1);mid+=weights[:,None]*np.array([0.,-C,S])*distance;expanded.append(mid.tolist());expanded_names.append('inferred convex rock masses' if belly else 'inferred irregular recessed joint')
 rings=expanded;names=expanded_names
 v=np.array(rings).reshape((-1,3)).tolist();n=len(rings[0]);faces=[]
 for row in range(len(rings)-1):
  for i in range(n):
   a=row*n+i;b=row*n+(i+1)%n;c=b+n;d=a+n;faces.extend([[a,c,b],[a,d,c]])
 # Ear clipping retains every exact boundary vertex on these concave caps.
 def cap(ring):
  pts=np.array(ring)[:,:2];ids=list(range(len(pts)));result=[]
  def cross(a,b,c):return float((b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0]))
  assert sum(cross(np.zeros(2),a,b)for a,b in zip(pts,np.roll(pts,-1,axis=0)))>0
  while len(ids)>3:
   for j,b in enumerate(ids):
    a=ids[j-1];c=ids[(j+1)%len(ids)]
    if cross(pts[a],pts[b],pts[c])<=1e-8:continue
    if any(all(t>=-1e-8 for t in [cross(pts[a],pts[b],pts[k]),cross(pts[b],pts[c],pts[k]),cross(pts[c],pts[a],pts[k])])for k in ids if k not in [a,b,c]):continue
    result.append([a,b,c]);ids.pop(j);break
   else:raise AssertionError('Concave cap triangulation failed')
  result.append(ids);return result
 for row,reverse in [(0,False),(len(rings)-1,True)]:
  for f in cap(rings[row]):
   ids=[row*n+i for i in f];faces.append(ids[::-1]if reverse else ids)
 oriented=Counter((a,b)for f in faces for a,b in zip(f,f[1:]+f[:1]));assert all(oriented[(b,a)]==1 for a,b in oriented)
 volume=sum(float(np.dot(v[f[0]],np.cross(v[f[1]],v[f[2]])))/6 for f in faces)
 if volume<0:faces=[f[::-1]for f in faces];volume=-volume
 ec=Counter(tuple(sorted((a,b)))for f in faces for a,b in zip(f,f[1:]+f[:1]));assert set(ec.values())=={2}and len(v)-len(ec)+len(faces)==2 and volume>0
 errors=[]
 for name in left:
  for p in traces[name]:
   native_point=np.array([p[0],-p[1]*S-p[2]*C]);projected=np.column_stack((np.array(v)[:,0],-np.array(v)[:,1]*S-np.array(v)[:,2]*C));errors.append(float(np.min(np.linalg.norm(projected-native_point,axis=1))))
 assert max(errors)<1e-7
 out={'vertices':v,'faces':faces,'volume':volume,'ring_names':names,'x_cut':565.,'source_anchor_error':max(errors),'previous_inferred_world':{name:traces[name]for name in left},'depth_provenance':'Coordinator-authorized correction of old sampled proxy heights; exact2D source anchors retained.','input_sha256':hashlib.sha256(inp.read_bytes()).hexdigest(),'status':'PRIVATE_CLOSED_EASTERN_COURSES_CPU_ONLY','limits':['Replacing eastern52 body requires saved cut/union and all original first-hit guards.','Source anchors are exact; hidden rear bulk and outer shoulders are inferred.','No source ownership, material or gameplay changes authorized by this candidate.']};(O/'geometry.json').write_text(json.dumps(out,indent=2)+'\n');print({'rings':len(rings),'vertices':len(v),'closed':True,'source_anchor_error':max(errors)})
if __name__=='__main__':main()
