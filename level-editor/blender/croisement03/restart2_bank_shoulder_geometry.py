"""Closed CPU trial retaining existing bank creases and adding observed shoulder edges."""
import json,math,hashlib,argparse
from collections import Counter
from pathlib import Path
import numpy as np
from restart2_bank_whole_builder import mesh
from restart2_bank_boundary_ray import boundary_hits
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement/restart2';O=B/'bank-shoulder-geometry-plan-v5';S=math.sin(math.radians(35));C=math.cos(math.radians(35));ray=np.array([0.,-C,S])

def planarize(boundary,lines):
 paths=[('boundary',boundary+[boundary[0]])]+lines
 segments=[];events={}
 for path_index,(_,points) in enumerate(paths):
  for segment_index,(a,b) in enumerate(zip(points,points[1:])):
   key=(path_index,segment_index);segments.append((key,np.array(a),np.array(b)));events[key]=[(0.,list(a)),(1.,list(b))]
 cross=lambda a,b:a[0]*b[1]-a[1]*b[0]
 for i,(ka,a,b) in enumerate(segments):
  delta=b[:2]-a[:2]
  for kb,c,d in segments[i+1:]:
   if ka[0]==kb[0] and abs(ka[1]-kb[1])<=1:continue
   other=d[:2]-c[:2];den=cross(delta,other)
   if abs(den)<1e-10:continue
   u=cross(c[:2]-a[:2],other)/den;v=cross(c[:2]-a[:2],delta)/den
   if -1e-9<=u<=1+1e-9 and -1e-9<=v<=1+1e-9:
    u=float(np.clip(u,0,1));v=float(np.clip(v,0,1));point=(a+u*(b-a)).tolist()
    events[ka].append((u,point));events[kb].append((v,point))
 rebuilt=[]
 for path_index,(name,points) in enumerate(paths):
  result=[]
  for segment_index in range(len(points)-1):
   ordered=sorted(events[(path_index,segment_index)],key=lambda x:x[0])
   merged=[]
   for u,p in ordered:
    if merged and abs(u-merged[-1][0])<1e-8:merged[-1]=(u,p)
    else:merged.append((u,p))
   for _,point in merged[:-1]:
    if not result or np.linalg.norm(np.array(point[:2])-result[-1][:2])>1e-8:result.append(point)
  result.append(points[-1]);rebuilt.append((name,result))
 return rebuilt[0][1][:-1],rebuilt[1:]

def main():
 global O
 ap=argparse.ArgumentParser();ap.add_argument('--output',type=Path,default=O);ap.add_argument('--recess',type=float,default=-2.);args=ap.parse_args();O=args.output.resolve()
 assert not O.exists();O.mkdir();old=json.loads((B/'bank-whole-geometry-plan-v4/geometry.json').read_text());source=json.loads((B/'bank-shoulder-source-v1/traces.json').read_text());result={};details=[]
 for owner in ('52','54'):
  m=old[owner];n=m['source_vertex_count'];verts=np.array(m['vertices']);top=[f for f in m['faces'] if all(i<n for i in f)];counts=Counter(tuple(sorted((a,b))) for f in top for a,b in zip(f,f[1:]+f[:1]));edges=[e for e,c in counts.items() if c==1];adj={}
  for a,b in edges:adj.setdefault(a,[]).append(b);adj.setdefault(b,[]).append(a)
  ring=[min(adj)];previous=None
  while True:
   next_=next(v for v in adj[ring[-1]] if v!=previous)
   if next_==ring[0]:break
   previous=ring[-1];ring.append(next_)
  assert len(ring)==len(adj)
  authored=lambda p:[float(p[0]),float(-p[1]*S),float(p[2]*C)]
  boundary=[authored(verts[i]) for i in ring]
  if owner=='52':
   # Extend the unprotected eastern top contour by1.66native pixels, retaining its height.
   a=ring.index(169);bb=ring.index(170);assert (a-bb)%len(ring) in (1,len(ring)-1)
   insert=max(a,bb) if abs(a-bb)==1 else len(boundary)
   height=verts[169,2]*C;assert abs(height-verts[170,2]*C)<1e-9
   boundary.insert(insert,[604.,189.+height,height])
  lines=[(t['id'],[authored(verts[i])for i in t['vertices']])for t in m['traces']]
  for t in source['traces']:
   if str(t['owner'])!=owner:continue
   points=[]
   for i,(x,sy) in enumerate(t['points']):
    original_source=[x,sy]
    if t['id']=='south-upper-short-shoulder' and i==4:x-=.5  # Within the traced shoulder's1–3pixel uncertainty, away from serialized right-wall edge.
    if t['id']=='east-upper-outer-shoulder' and i==0:
     points.append([604.,189.+height,height]);continue
    origin=np.array([x,-sy/S,0.])+ray*5000;hits=boundary_hits(origin,-ray,{owner:(verts,m['faces'])});assert hits,(t['id'],i,'no existing surface')
    surface=origin-ray*hits[0]['distance'];shift=args.recess if i in (3,5) else 0.;point=surface+ray*shift;points.append(authored(point));details.append({'trace':t['id'],'sample':i,'source':[x,sy],'ray_shift_world':shift,'world':point.tolist()})
   lines.append((t['id'],points))
  boundary,lines=planarize(boundary,lines)
  candidate=mesh(boundary,lines);candidate['native_obstacle']=m['native_obstacle'];candidate['gameplay_metadata']=m['gameplay_metadata'];result[owner]=candidate
 result['guards']=old['guards'];result['shoulder_source_sha256']=hashlib.sha256((B/'bank-shoulder-source-v1/traces.json').read_bytes()).hexdigest();result['scope']='CPU trial; small recessed joints along traced shoulders. Saved source-firsthit and appearance guards required.'
 (O/'geometry.json').write_text(json.dumps(result,indent=2)+'\n');(O/'shoulder-points.json').write_text(json.dumps(details,indent=2)+'\n');print({k:{x:result[k][x]for x in ('source_vertex_count','signed_volume','euler')}for k in ('52','54')})
if __name__=='__main__':main()
