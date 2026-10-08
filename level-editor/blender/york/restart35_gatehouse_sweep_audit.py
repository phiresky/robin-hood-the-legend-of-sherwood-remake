"""Read-only CPU triangle-intersection audit against the pinned full gatehouse."""
import hashlib,json,struct
from pathlib import Path
import numpy as np
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/york-refinement/restart2';OUT=W/'approved-shed-jamb-motion-integration-v1/gatehouse-sweep-audit.json';assert not OUT.exists()
packet=json.loads((OUT.parent/'integration.json').read_text());asset=ROOT/'level-editor/library/3d-assets/york/york-castle-west-gatehouse';path=asset/'model.glb';blob=path.read_bytes();sha=hashlib.sha256(blob).hexdigest();assert sha==packet['protected_live_inputs'][str(path)]
magic,version,length=struct.unpack_from('<III',blob);assert magic==0x46546c67 and version==2 and length==len(blob);offset=12;binary=None
while offset<len(blob):
 size,kind=struct.unpack_from('<II',blob,offset);offset+=8;chunk=blob[offset:offset+size];offset+=size
 if kind==0x4e4f534a:gltf=json.loads(chunk)
 elif kind==0x004e4942:binary=chunk
assert binary is not None
# The map root is the standard Z-up to glTF basis. Raw child positions retain
# authored scene coordinates relative to source_origin_scene.
root=next(x for x in gltf['nodes']if x['name']=='map');assert np.allclose(root['rotation'],[-2**-.5,0,0,2**-.5])
for node in gltf['nodes']:
 if node is not root:assert not any(k in node for k in ['matrix','rotation','translation','scale'])
pivot=np.array(json.loads((asset/'asset.json').read_text())['source_origin_scene'])
def accessor(index):
 a=gltf['accessors'][index];assert not a.get('sparse') and not a.get('normalized');v=gltf['bufferViews'][a['bufferView']];assert v['buffer']==0;kind={5126:'<f4',5125:'<u4',5123:'<u2'}[a['componentType']];width={'VEC3':3,'SCALAR':1}[a['type']];dt=np.dtype(kind);start=v.get('byteOffset',0)+a.get('byteOffset',0);stride=v.get('byteStride',dt.itemsize*width);return np.ndarray((a['count'],width),dtype=dt,buffer=binary,offset=start,strides=(stride,dt.itemsize)).copy()
parts=[]
for node in gltf['nodes']:
 if 'mesh' not in node:continue
 tris=[]
 for primitive in gltf['meshes'][node['mesh']]['primitives']:
  assert primitive.get('mode',4)==4;vs=accessor(primitive['attributes']['POSITION'])+pivot;ids=accessor(primitive['indices']).ravel().reshape(-1,3);tris.extend(vs[ids].tolist())
 tris=np.array(tris);parts.append({'source_node':node['extras']['source_node'],'triangles':tris})
audit=json.loads((W/'gate-saved-contact-audit-v1/report.json').read_text());gv=np.array(audit['geometry_world']['gate_vertices']);gf=audit['geometry_world']['gate_faces'];gids=[(f[0],f[i],f[i+1])for f in gf for i in range(1,len(f)-1)];motion=json.loads((W/'jamb-clearance-candidate-v1/motion-proposal.json').read_text())
def segment_triangle(p,q,tri):
 e1=tri[:,1]-tri[:,0];e2=tri[:,2]-tri[:,0];direction=q-p;h=np.cross(direction,e2);det=np.einsum('ij,ij->i',e1,h);valid=np.abs(det)>1e-9;inv=np.zeros_like(det);inv[valid]=1/det[valid];s=p-tri[:,0];u=np.einsum('ij,ij->i',s,h)*inv;r=np.cross(s,e1);v=r@direction*inv;t=np.einsum('ij,ij->i',e2,r)*inv;hits=valid&(u>=-1e-7)&(v>=-1e-7)&(u+v<=1+1e-7)&(t>1e-6)&(t<1-1e-6);return np.where(hits)[0]
rows=[]
for row in motion['rows']:
 gate=gv+np.array([0,0,row['nominal_lift_world_z']]);gt=gate[gids];hits=[]
 for part in parts:
  tr=part['triangles'];mn=tr.min(axis=1);mx=tr.max(axis=1);pairs=[]
  for gi,t in enumerate(gt):
   possible=np.where(np.all(mx>=t.min(axis=0)-1e-6,axis=1)&np.all(mn<=t.max(axis=0)+1e-6,axis=1))[0]
   if not len(possible):continue
   indices=set()
   for i,j in [(0,1),(1,2),(2,0)]:indices.update(int(possible[k])for k in segment_triangle(t[i],t[j],tr[possible]))
   for ci in possible:
    if ci in indices:continue
    for i,j in [(0,1),(1,2),(2,0)]:
     if len(segment_triangle(tr[ci,i],tr[ci,j],t[None])):indices.add(int(ci));break
   pairs.extend([gi,ci]for ci in sorted(indices))
  if pairs:hits.append({'source_node':part['source_node'],'triangle_pair_count':len(pairs),'examples':pairs[:8]})
 rows.append({'frame':row['frame'],'lift_world_z':row['nominal_lift_world_z'],'confirmed_surface_intersections':hits})
report={'status':'READ_ONLY_FULL_GATEHOUSE_TRIANGLE_CROSSING_AUDIT','model_sha256':sha,'source_origin_scene':pivot.tolist(),'parts':[{'source_node':p['source_node'],'triangles':len(p['triangles'])}for p in parts],'rows':rows,'poses_with_confirmed_crossings':sum(bool(r['confirmed_surface_intersections'])for r in rows),'limits':['This is separate from the approved gate-to-jamb contact scope.','Strict segment/triangle crossings confirm surface intersection; coplanar contact and complete containment are not certified by this audit.','No model, runtime, source art or canonical file was changed.']};OUT.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'frames':45,'poses_with_crossings':report['poses_with_confirmed_crossings'],'parts':sorted(set(h['source_node']for r in rows for h in r['confirmed_surface_intersections']))}))
