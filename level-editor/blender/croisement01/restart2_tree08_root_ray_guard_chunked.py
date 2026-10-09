"""Checkpoint the strict intersection predicate in bounded disjoint face chunks."""
import hashlib,json,sys
from pathlib import Path
import numpy as np
from vtkmodules.vtkCommonCore import vtkIdList
from vtkmodules.vtkCommonDataModel import vtkStaticCellLocator
from restart2_tree08_fork_kernel import poly
from restart2_tree08_local_fork import audit
from restart2_tree08_junction_proof import intersection,precise_plane_separation
R=Path(__file__).resolve().parents[3]/'level-editor/work/croisement01-refinement/restart2';packet=R/(sys.argv[sys.argv.index('--packet')+1] if '--packet' in sys.argv else ('tree08-root-ray-cpu-v4' if '--fit' in sys.argv else 'tree08-root-ray-cpu-v3'));assert packet.resolve().parent==R.resolve();data=np.load(packet/'candidate.npz');vertices,faces,before=data['vertices'],data['faces'],data['before_vertices'];topology=audit(vertices,faces);original=audit(before,faces);assert not any(topology[k] for k in ['nonmanifold_edges','winding_errors','zero_area']);changed=np.any(np.any(vertices[faces]!=before[faces],axis=2),axis=1);triangles=vertices[faces];old=before[faces];lower=triangles.min(1);upper=triangles.max(1);locator=vtkStaticCellLocator();locator.SetDataSet(poly(vertices,faces));locator.BuildLocator();centers=triangles.mean(1);inset=centers[:,None]+(triangles-centers[:,None])*(1-1e-7);seen=set();new=[];retained=[];tested=0
chunk_size=2000
candidate_sha=hashlib.sha256((packet/'candidate.npz').read_bytes()).hexdigest()
changed_ids=np.flatnonzero(changed)
proofdir=packet/'intersection-chunks-v1';proofdir.mkdir(exist_ok=True)
chunk_index=int(sys.argv[sys.argv.index('--chunk')+1])
assert 0<=chunk_index<(len(changed_ids)+chunk_size-1)//chunk_size
start=chunk_index*chunk_size;stop=min(start+chunk_size,len(changed_ids))
proofpath=proofdir/f'chunk-{chunk_index:03d}.json';assert not proofpath.exists()
for i in changed_ids[start:stop]:
 if i%2000==0:print('ROOT INTERSECTION FACE',int(i),flush=True)
 lo,hi=lower[i],upper[i];ids=vtkIdList();locator.FindCellsWithinBounds([lo[0],hi[0],lo[1],hi[1],lo[2],hi[2]],ids)
 for k in range(ids.GetNumberOfIds()):
  j=ids.GetId(k);pair=tuple(sorted((int(i),int(j))))
  if i==j or (changed[j] and j<i) or np.any(upper[j]<lo-1e-9) or np.any(lower[j]>hi+1e-9):continue
  shared=set(faces[i])&set(faces[j]);adjacent=bool(shared)
  if len(shared)==2:
   a,b=triangles[i],triangles[j];n1=np.cross(a[1]-a[0],a[2]-a[0]);n2=np.cross(b[1]-b[0],b[2]-b[0]);angle=np.linalg.norm(np.cross(n1/np.linalg.norm(n1),n2/np.linalg.norm(n2)))
   if angle>1e-12:continue
  a,b=(inset[i],inset[j]) if adjacent else (triangles[i],triangles[j])
  if np.any(a.max(0)<b.min(0)) or np.any(b.max(0)<a.min(0)):continue
  tested+=1
  if not intersection(a,b) or precise_plane_separation(triangles[i],triangles[j],adjacent):continue
  oa,ob=old[i],old[j]
  if adjacent:oa=oa.mean(0)+(oa-oa.mean(0))*(1-1e-7);ob=ob.mean(0)+(ob-ob.mean(0))*(1-1e-7)
  bucket=retained if intersection(oa,ob) and not precise_plane_separation(old[i],old[j],adjacent) else new;bucket.append(dict(faces=list(pair),adjacent=adjacent))
report=dict(status='PASS_NO_NEW_INTERSECTIONS' if not new else 'FAIL_NEW_INTERSECTIONS',candidate_sha256=hashlib.sha256((packet/'candidate.npz').read_bytes()).hexdigest(),topology=topology,original_topology=original,changed_faces=int(changed.sum()),tested_pairs=tested,new_intersections=new,retained_intersections=retained,scope='All candidate bbox pairs touching changed faces, unchanged geometry retains previous proof; existing independent pieces must remain disclosed.')
report.update(chunk_index=chunk_index,chunk_start=start,chunk_stop=stop,total_chunks=(len(changed_ids)+chunk_size-1)//chunk_size,chunk_size=chunk_size,guard_recipe_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest())
proofpath.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2),flush=True)

proofs=sorted(proofdir.glob('chunk-*.json'))
if len(proofs)==report['total_chunks']:
 chunks=[json.loads(p.read_text()) for p in proofs]
 assert [c['chunk_index'] for c in chunks]==list(range(report['total_chunks']))
 assert all(c['candidate_sha256']==candidate_sha and c['guard_recipe_sha256']==report['guard_recipe_sha256'] for c in chunks)
 assert all(c['chunk_start']==i*chunk_size and c['chunk_stop']==min((i+1)*chunk_size,len(changed_ids)) for i,c in enumerate(chunks))
 merged=dict(report)
 for key in ['chunk_index','chunk_start','chunk_stop']:merged.pop(key)
 merged['new_intersections']=[x for c in chunks for x in c['new_intersections']]
 merged['retained_intersections']=[x for c in chunks for x in c['retained_intersections']]
 merged['tested_pairs']=sum(c['tested_pairs'] for c in chunks)
 merged['status']='FAIL_NEW_INTERSECTIONS' if merged['new_intersections'] else 'PASS_NO_NEW_INTERSECTIONS'
 merged['scope']='All bbox pairs touching changed faces, assigned once to their smaller changed face; strict predicates unchanged. Unchanged geometry retains prior proof.'
 merged['chunk_sha256']={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in proofs}
 (packet/'intersection-guard.json').write_text(json.dumps(merged,indent=2)+'\n')
 print('MERGED',merged['status'],flush=True)
