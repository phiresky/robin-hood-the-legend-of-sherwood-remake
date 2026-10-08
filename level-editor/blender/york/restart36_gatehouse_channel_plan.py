"""Read-only CPU triangle-intersection audit against the pinned full gatehouse."""
import hashlib,json,struct
from pathlib import Path
import numpy as np
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/york-refinement/restart2';OUT=W/'approved-shed-jamb-motion-integration-v1/gatehouse-channel-proposal.json';assert not OUT.exists()
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

audit=json.loads((W/'gate-saved-contact-audit-v1/report.json').read_text());gv=np.array(audit['geometry_world']['gate_vertices']);base=np.array(audit['geometry_world']['jamb_vertices'][0]);proposal=json.loads((W/'jamb-hidden-clearance-plan-v2/proposal.json').read_text());normal=np.array(proposal['normal_world']);axis=np.array([-normal[1],normal[0],0]);motion=json.loads((W/'jamb-clearance-candidate-v1/motion-proposal.json').read_text());lift=max(r['nominal_lift_world_z']for r in motion['rows']);coords=np.column_stack([(gv-base)@axis,(gv-base)@normal,gv[:,2]]);lo=coords.min(axis=0)-.05;hi=coords.max(axis=0)+.05;hi[2]+=lift
# Evaluate whether removing this continuous swept envelope would touch currently
# source-facing surfaces; full-context rays are conservative, not artwork ownership.
s=np.sin(np.deg2rad(35));c=np.cos(np.deg2rad(35));back=np.array([0,-c,s]);ray=-back;pixels=np.array([[x+.5,y+.5]for y in range(780,1030)for x in range(2250,2470)]);origins=np.column_stack([pixels[:,0],-pixels[:,1]/s,np.zeros(len(pixels))])+back*10000;nearest=np.full(len(pixels),np.inf);owners=np.full(len(pixels),-1);triowners=np.full(len(pixels),-1)
for pi,part in enumerate(parts):
 for ti,(a,b,d)in enumerate(part['triangles']):
  e1=b-a;e2=d-a;h=np.cross(ray,e2);det=e1@h
  if abs(det)<1e-10:continue
  q=origins-a;u=q@h/det;cross=np.cross(q,e1);v=cross@ray/det;t=cross@e2/det;hit=(u>=-1e-7)&(v>=-1e-7)&(u+v<=1+1e-7)&(t>=0)&(t<nearest);nearest[hit]=t[hit];owners[hit]=pi;triowners[hit]=ti
valid=np.isfinite(nearest);points=origins[valid]+nearest[valid,None]*ray;local=np.column_stack([(points-base)@axis,(points-base)@normal,points[:,2]]);removed=np.all(local>lo,axis=1)&np.all(local<hi,axis=1);affected=[{'pixel':[int(x),int(y)],'source_node':parts[int(oi)]['source_node'],'triangle':int(ti),'world':p.tolist()}for (x,y),oi,ti,p,hit in zip(pixels[valid],owners[valid],triowners[valid],points,removed)if hit]
# Count channel intersection with proxy triangles; retain exact mesh faces outside it.
component_summary=[]
for part in parts:
 v=part['triangles'].reshape(-1,3);q=np.column_stack([(v-base)@axis,(v-base)@normal,v[:,2]]).reshape(-1,3,3);candidate=np.all(q.max(axis=1)>=lo,axis=1)&np.all(q.min(axis=1)<=hi,axis=1);component_summary.append({'source_node':part['source_node'],'triangles':len(q),'channel_aabb_candidate_triangles':np.where(candidate)[0].tolist(),'bounds_channel_basis':[q.min(axis=(0,1)).tolist(),q.max(axis=(0,1)).tolist()]})
report={'status':'CPU_CHANNEL_PROPOSAL_NOT_GEOMETRY_APPROVED_OR_SAVED','protected_gate_model_sha256':audit['model_sha256'],'protected_jamb_model_sha256':json.loads((W/'jamb-clearance-candidate-v1/validation.json').read_text())['saved_model_sha256'],'source_glb_sha256':sha,'channel_basis':{'origin':base.tolist(),'horizontal_axis':axis.tolist(),'normal':normal.tolist(),'z':'absolute world Z'},'continuous_swept_channel_bounds':[lo.tolist(),hi.tolist()],'components':component_summary,'native_crop':[2250,780,2470,1030],'conservative_full_context_first_hits':int(valid.sum()),'first_hits_inside_channel':affected,'diagnosis':'Existing778/779 are grounded14triangle prismatic volumes, not source-traced arch or sliding-guide meshes. Their intersections cannot justify changing the approved gate motion. A narrow vertical guide channel cut from these inferred solids is the first correction hypothesis.','geometry_recipe':'Subtract the oriented continuous swept channel from scoped778/779 only; retain all outside vertices, source mappings and other gatehouse parts. Cap new unknown interior faces as separate inferred surfaces. Preserve the approved jamb as an independent component, not a replacement for the whole778volume.','source_guard_limit':'First-hit test covers current proxy surfaces only. Any affected pixels must be independently classified against art, masks and patch ownership before a model change. Even zero affected proxy rays would still need saved-model source, solid, contact and unknown-surface review.','approval_scope':'New hidden recess/guide geometry on778/779, separate from already approved gate/jamb pair and45poses.'};OUT.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'native_first_hits':int(valid.sum()),'inside_channel':len(affected),'affected_parts':sorted(set(r['source_node']for r in affected)),'bounds':[lo.tolist(),hi.tolist()]}))
