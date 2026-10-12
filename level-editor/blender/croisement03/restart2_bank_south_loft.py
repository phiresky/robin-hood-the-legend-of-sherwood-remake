"""Narrow the southern rock body around measured strata, not inferred guard wings."""
import json,math,hashlib
from pathlib import Path
from collections import Counter
import numpy as np
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement/restart2';O=B/'bank-shoulder-lofts-cpu-v6';S=math.sin(math.radians(35));C=math.cos(math.radians(35))
def cap(points):
 p=np.array(points)[:,:2];ids=list(range(len(p)));out=[]
 def cross(a,b,c):return float((b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0]))
 assert sum(cross(np.zeros(2),a,b)for a,b in zip(p,np.roll(p,-1,axis=0)))>0
 while len(ids)>3:
  for j,b in enumerate(ids):
   a=ids[j-1];c=ids[(j+1)%len(ids)]
   if cross(p[a],p[b],p[c])<=1e-8:continue
   if any(all(x>=-1e-8 for x in [cross(p[a],p[b],p[k]),cross(p[b],p[c],p[k]),cross(p[c],p[a],p[k])])for k in ids if k not in [a,b,c]):continue
   out.append([a,b,c]);ids.pop(j);break
  else:raise AssertionError('Cap triangulation failed')
 out.append(ids);return out

def main():
 O.mkdir(exist_ok=False);inp=B/'bank-sidewall-cpu-v1/geometry.json';data=json.loads(inp.read_text());old=data['54'];plan=json.loads((B/'bank-whole-source-plan-v1/plan.json').read_text());native={t['id']:t for t in plan['new_traces']if t['owner']==54};rows=[];provenance=[];fractions={0.,1.}
 for t in old['traces']:
  kind='upper'if t['id'].endswith('-upper')else'lower';name=t['id'].rsplit('-',1)[0];expected=[p['source_'+kind]for p in native[name]['paired_world']];ids=t['vertices'][1:-1];old_points=[old['vertices'][i]for i in ids];points=old_points;projected=[[v[0],-v[1]*S-v[2]*C]for v in points];assert np.max(np.abs(np.array(projected)-expected))<1e-6
  height={'south-upper-short-ledge':62.,'south-middle-short-ledge':42.,'south-low-exposed-tip':18.}[name];base_source=native[name]['paired_world'];points=[[p[0],-(p[1]+height+(1.5 if kind=='lower' else 0.))/S,(height+(1.5 if kind=='lower' else 0.)-(pair['source_lower'][1]-pair['source_upper'][1] if kind=='lower' else 0.))/C]for p,pair in zip([pair['source_upper']for pair in base_source],base_source)];left=points[0][0]-3;right=points[-1][0]+(5 if 'middle' in t['id'] else 3);fractions.update((p[0]-left)/(right-left)for p in points);rows.append({'name':t['id'],'points':points,'left':left,'right':right});provenance.append({'trace':t['id'],'observed_core':projected,'previous_inferred_world':old_points,'revised_inferred_world':points,'depth_provenance':'Old-model sampled anchor height is an inference; exact native upper/lower positions retained.','reclassified_inferred_extensions':[{'vertex':i,'world':old['vertices'][i],'native':[old['vertices'][i][0],-old['vertices'][i][1]*S-old['vertices'][i][2]*C],'origin':'extend_stripes generated shoulder continuation, not measured artwork'}for i in [t['vertices'][0],t['vertices'][-1]]]})
 fractions=sorted(fractions);rings=[];labels=[];nfront=len(fractions)
 # The top strip remains wholly forward of the exact shared52/54 ridge.
 a,b=[np.array([x,-y/S,z/C])for x,y,z in data['guards']['interface52_54']];front=[(a*(1-t)+b*t+np.array([0.,-3/S,0.])).tolist()for t in fractions];rings.append(front+[b.tolist(),((a+b)/2).tolist(),a.tolist()]);labels.append('fixed shared ridge strip')
 for row in rows:
  pts=np.array(row['points']);xs=[row['left']]+list(pts[:,0])+[row['right']];ys=[pts[0,1]]+list(pts[:,1])+[pts[-1,1]];zs=[pts[0,2]]+list(pts[:,2])+[pts[-1,2]];front=[[x,float(np.interp(x,xs,ys)),float(np.interp(x,xs,zs))]for x in [row['left']+t*(row['right']-row['left'])for t in fractions]];center=np.mean(front,axis=0);back=[[row['right']-1,front[-1][1]+12/S,front[-1][2]],[center[0],center[1]+25/S,center[2]],[row['left']+1,front[0][1]+12/S,front[0][2]]];rings.append(front+back);labels.append(row['name'])
 front=[[350+t*29,-float(np.interp(t,[0,.5,1],[449,457,450]))/S,0.]for t in fractions];rings.append(front+[[378,-433/S,0.],[364.5,-426/S,0.],[351,-433/S,0.]]);labels.append('inferred narrow grounded closure')
 expanded=[];expanded_labels=[]
 for i,ring in enumerate(rings):
  expanded.append(ring);expanded_labels.append(labels[i])
  if i in (1,2,3,4,5):
   a=np.array(ring);b=np.array(rings[i+1]);gap=float(np.min(a[:,2]-b[:,2]));assert gap>0;belly=i in (1,3,5);distance=min(3. if belly else 5.,gap*.25/S)*(1 if belly else -1);mid=(a+b)/2+np.array([0.,-C,S])*distance;expanded.append(mid.tolist());expanded_labels.append('inferred rounded course' if belly else 'inferred recessed riser')
 rings=expanded;labels=expanded_labels
 v=np.array(rings).reshape((-1,3)).tolist();n=len(rings[0]);faces=[]
 for row in range(len(rings)-1):
  for i in range(n):
   a=row*n+i;b=row*n+(i+1)%n;c=b+n;d=a+n;faces.extend([[a,c,b],[a,d,c]])
 for row,reverse in [(0,False),(len(rings)-1,True)]:
  for f in cap(rings[row]):
   ids=[row*n+i for i in f];faces.append(ids[::-1]if reverse else ids)
 counts=Counter(tuple(sorted((a,b)))for f in faces for a,b in zip(f,f[1:]+f[:1]));oriented=Counter((a,b)for f in faces for a,b in zip(f,f[1:]+f[:1]));assert set(counts.values())=={2}and all(oriented[(b,a)]==1 for a,b in oriented);assert len(v)-len(counts)+len(faces)==2;volume=sum(float(np.dot(v[f[0]],np.cross(v[f[1]],v[f[2]])))/6 for f in faces);assert volume>0
 traces=[];errors=[]
 for row in rows:
  ids=[]
  for point in row['points']:
   ds=np.linalg.norm(np.array(v)-point,axis=1);idx=int(np.argmin(ds));errors.append(float(ds[idx]));ids.append(idx)
  traces.append({'id':row['name'],'vertices':ids})
 assert max(errors)<1e-7
 east_provenance=[]
 for trace in data['52']['traces']:
  if not trace['id'].startswith('east-'):continue
  height={'east-upper-block-upper':75.,'east-upper-block-lower':67.5,'east-middle-block-upper':54.,'east-middle-block-lower':43.5,'east-low-block-upper':34.,'east-low-block-lower':21.5}[trace['id']]
  for idx in trace['vertices']:
   old_point=data['52']['vertices'][idx];x,y,z=old_point;sy=-y*S-z*C;new=[x,-(sy+height)/S,height/C];assert abs(-new[1]*S-new[2]*C-sy)<1e-7;data['52']['vertices'][idx]=new;east_provenance.append({'trace':trace['id'],'source':[x,sy],'old_inferred_world':old_point,'new_inferred_world':new})
 data['east_depth_provenance']=east_provenance
 data['54']={'vertices':v,'faces':faces,'traces':traces,'signed_volume':volume,'edge_count':len(counts),'euler':2,'native_obstacle':54,'gameplay_metadata':old['gameplay_metadata'],'topology':'Closed connected full-body rock loft with observed strata and a narrow inferred lower shoulder.','ring_names':labels};data['guard_provenance']={'previous_geometry_sha256':hashlib.sha256(inp.read_bytes()).hexdigest(),'correction_authority':'Coordinator authorized correcting sampled old-model trace depths and generated extensions as hypotheses; preserve exact2D observed creases, independently supported shared crest/contact and neighbor constraints.','traces':provenance,'observed_anchor_error':max(errors),'old_inferred_extensions_are_not_measured':True};(O/'geometry.json').write_text(json.dumps(data,indent=2)+'\n');(O/'guard-provenance.json').write_text(json.dumps(data['guard_provenance'],indent=2)+'\n');print({'vertices':len(v),'faces':len(faces),'source_anchor_error':max(errors),'generated_endpoints_reclassified':12})
if __name__=='__main__':main()
