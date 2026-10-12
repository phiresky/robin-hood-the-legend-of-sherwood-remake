"""Shape coherent recessed risers between exact source-supported rock creases."""
import json,math,hashlib
from pathlib import Path
from collections import defaultdict,Counter
import numpy as np
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/active/croisement03/restart2';O=B/'bank-strata-relief-cpu-v1';S=math.sin(math.radians(35));C=math.cos(math.radians(35));ray=np.array([0.,-C,S])
def main():
 inp=B/'bank-sidewall-cpu-v1/geometry.json';data=json.loads(inp.read_text());O.mkdir(exist_ok=False);report=[]
 for owner in ['52','54']:
  m=data[owner];v=np.array(m['vertices']);faces=m['faces'];n=m['source_vertex_count'];top=[all(i<n for i in f)for f in faces];segments=[(v[a],v[b])for t in m['traces']for a,b in zip(t['vertices'],t['vertices'][1:])];bound=set(m['boundary_top_indices']);source=v.copy()
  # Split both sides of every shared edge so the closed volume remains manifold.
  for iteration in range(2):
   points=v.tolist();mid={};new=[];nt=[]
   def split(a,b):
    e=tuple(sorted((a,b)))
    if e not in mid:mid[e]=len(points);points.append(((v[a]+v[b])/2).tolist())
    return mid[e]
   for f,t in zip(faces,top):
    a,b,c=f;ab=split(a,b);bc=split(b,c);ca=split(c,a);new.extend([[a,ab,ca],[ab,b,bc],[ca,bc,c],[ab,bc,ca]]);nt += [t]*4
   v=np.array(points);faces=new;top=nt
  edgefaces=defaultdict(list)
  for fi,f in enumerate(faces):
   for a,b in zip(f,f[1:]+f[:1]):edgefaces[tuple(sorted((a,b)))].append(fi)
  topids={i for f,t in zip(faces,top)if t for i in f};fixed={i for e,fs in edgefaces.items()if not all(top[fi]for fi in fs)for i in e};sp=np.array([[p[0],-p[1]*S-p[2]*C]for p,q in segments]);sq=np.array([[q[0],-q[1]*S-q[2]*C]for p,q in segments]);vectors=sq-sp;lengths=np.sum(vectors*vectors,axis=1);moves=[]
  for i in sorted(topids-fixed):
   p=v[i];screen=np.array([p[0],-p[1]*S-p[2]*C]);ts=np.clip(np.sum((screen-sp)*vectors,axis=1)/lengths,0,1);distance=float(np.min(np.linalg.norm(screen-(sp+ts[:,None]*vectors),axis=1)))
   # Protect full source crease segments, not only their sampled vertices.
   if distance<1e-5:continue
   # Source-exposed eastern shoulder and southern rock slope only. The shared
   # crest and retained west53 remain outside the sculpted domain.
   weight=np.clip((p[0]-565)/25,0,1)if owner=='52'else np.clip((screen[1]-332)/20,0,1)
   amount=float(min(8.,.6*distance)*weight);amount=min(amount,max(0.,p[2]*.15))
   if amount>1e-6:v[i]-=ray*amount;moves.append(amount)
  # Exact source coordinates survive; rings and silhouettes are never inflated.
  for t in m['traces']:
   for i in t['vertices']:assert np.max(np.abs(v[i]-source[i]))<1e-7
  for i in m['boundary_top_indices']:assert np.array_equal(v[i],source[i])
  ec=Counter(tuple(sorted((a,b)))for f in faces for a,b in zip(f,f[1:]+f[:1]));assert set(ec.values())=={2}and len(v)-len(ec)+len(faces)==2;volume=sum(float(np.dot(v[f[0]],np.cross(v[f[1]],v[f[2]])))/6 for f in faces);assert volume>0
  m.update(vertices=v.tolist(),faces=faces,signed_volume=volume,edge_count=len(ec),euler=2,topology='Closed continuous body with source-fixed strata, inward-curved risers and unchanged shared boundary.');report.append({'owner':owner,'vertices':len(v),'faces':len(faces),'moved':len(moves),'max_recess':max(moves,default=0),'boundary_exact':True,'source_creases_exact':True})
 data['relief_revision']={'input_sha256':hashlib.sha256(inp.read_bytes()).hexdigest(),'owners':report,'limits':['Private morphology hypothesis; saved topology, first-hit ownership and actual eight-view review required.','No source pixel ownership or textures added.']};(O/'geometry.json').write_text(json.dumps(data,indent=2)+'\n');print(report)
if __name__=='__main__':main()
