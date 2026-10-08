"""Compare source-facing roots with hash-bound current receiver geometry on CPU."""
import hashlib,json,math,struct
from pathlib import Path
import numpy as np
from PIL import Image
from restart2_tree08_junction_proof import native_depth
ROOT=Path(__file__).resolve().parents[3];R=ROOT/'level-editor/work/croisement01-refinement/restart2';lib=ROOT/'level-editor/library';out=R/'tree08-receiver-depth-cpu-v1';out.mkdir(exist_ok=False)
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def raw_mesh(asset):
 path=lib/asset['model'];assert sha(path)==asset['model_sha256'];b=path.read_bytes();n,kind=struct.unpack_from('<II',b,12);assert kind==0x4e4f534a;g=json.loads(b[20:20+n]);buffers=[]
 for buf in g['buffers']:
  if 'uri' in buf:buffers.append((path.parent/buf['uri']).resolve().read_bytes())
  else:buffers.append(b[28+n:])
 def read(index):
  a=g['accessors'][index];v=g['bufferViews'][a['bufferView']];dtype={5126:'<f4',5125:'<u4',5123:'<u2'}[a['componentType']];width={'VEC3':3,'SCALAR':1}[a['type']];offset=v.get('byteOffset',0)+a.get('byteOffset',0);stride=v.get('byteStride',np.dtype(dtype).itemsize*width)
  return np.ndarray((a['count'],width),dtype=dtype,buffer=buffers[v['buffer']],offset=offset,strides=(stride,np.dtype(dtype).itemsize)).copy()
 assert len(g['meshes'])==1 and len(g['meshes'][0]['primitives'])==1
 primitive=g['meshes'][0]['primitives'][0];return read(primitive['attributes']['POSITION']).astype(float),read(primitive['indices']).reshape(-1,3).astype(int)
scene_path=lib/'scenes/croisement01.rhlos-map.json';scene=json.loads(scene_path.read_text());s,c=np.sin(np.radians(35)),np.cos(np.radians(35));receivers=[];binding=[]
for asset in [scene['sceneAssets'][0],next(a for a in scene['assetSources'] if a['id']=='croisement01-terrace-000')]:
 v,f=raw_mesh(asset);translation=np.zeros(3);placement=next((p for p in scene['placements'] if p['assets']==[asset['id']]),None)
 if placement:
  transforms=[placement['transform']]+[p['transform'] for p in placement.get('parts',{}).values()];assert len(transforms)<=2
  for t in transforms:assert t['rot_deg']==0;translation+=np.array([t['dx'],-t['dy']/s,t['dz']/c])
 v+=translation;receivers.append((v,f));binding.append(dict(asset=asset,translation=translation.tolist(),bounds=[v.min(0).tolist(),v.max(0).tolist()]))
source=R/'tree08-v12-chain-cpu-v3';audit=json.loads((source/'report.json').read_text());a=np.load(source/'mesh.npz');plan=json.loads((source/'fork-union-plan.json').read_text());used={i for g in plan['groups'] for i in g};sections=[];origin=np.array([552.,-672.,235.]);packets=['tree08-v12-remaining-group0-stitched-v3-conformed-stable-depth-corrected','tree08-v12-remaining-group1-stitched-v2-stable'];hashes={}
for packet in packets:
 path=R/packet/'candidate.npz';m=np.load(path);sections.append((m['vertices'],m['faces']));hashes[str(path)]=sha(path)
for entry in audit['mesh_sections']:
 i=entry['index']
 if i not in used:sections.append((a[f'vertices_{i}'],a[f'faces_{i}']))
sections=[((v-origin).astype(np.float32).astype(float)+origin,f) for v,f in sections];wood=native_depth(sections);ground=native_depth(receivers[:1]);terrace=native_depth(receivers[1:]);support=np.maximum(ground,terrace);core=np.asarray(Image.open(R/'tree08-semantic-source-v1/bark-core-proposal.png'))>0;assert int(core.sum())==6276 and np.isfinite(wood[core]).all();hidden=core&(support>wood+1e-4);yy,xx=np.mgrid[:461,:446];root=yy+11>=340;delta=np.where(np.isfinite(wood),np.maximum(0,support-wood),0);np.savez_compressed(out/'native-depth.npz',wood=wood,ground=ground,terrace=terrace,required_ray_delta=delta,core=core)
anchors=[]
for a in json.loads((R/'tree08-topology-plan-v1/plan.json').read_text())['root_terrain_anchors']:
 x,y=a['native'];ix,iy=x-331,y-11;d=delta[iy,ix];anchors.append(dict(anchor=a,pixel_center=[x+.5,y+.5],receiver='terrace' if terrace[iy,ix]>ground[iy,ix] else 'ground',wood_pixel_hit=bool(np.isfinite(wood[iy,ix])),required_ray_delta=float(d) if np.isfinite(wood[iy,ix]) else None,world_delta=[0.,float(-c*d),float(s*d)] if np.isfinite(wood[iy,ix]) else None))
report=dict(status='DEPTH_HYPOTHESIS_ONLY',scene_sha256=sha(scene_path),wood_meshes=hashes,receiver_bindings=binding,core_pixels=6276,occluded_core_pixels=int(hidden.sum()),root_core_pixels=int((core&root).sum()),occluded_root_core_pixels=int((hidden&root).sum()),anchors=anchors,source_authority=['Native root image has a continuous visible flare and descending roots A-E, so fully buried construction is inconsistent with the artwork.','Terrace top/sides are existing authored receiver approximations derived from obstacle boundaries; occlusion and movement heights alone do not specify soil microtopography.','Root wood depth is unapproved inference. Preserve current approved receiver geometry and known source pixels while testing wood displacement only along native sight rays.','This is not authority to transfer ambiguous shadow/leaf pixels onto wood, or to change approved terrain.'],next='Evaluate a smooth source-ray depth field for unapproved lower wood, leaving upper approved/held assets untouched. Positive clearance upper surface and embedded underside require separate checks; native front-depth changes must be explicit, not hidden by relaxing union-preservation guards.')
(out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:report[k] for k in ['core_pixels','occluded_core_pixels','root_core_pixels','occluded_root_core_pixels','anchors']},indent=2))
