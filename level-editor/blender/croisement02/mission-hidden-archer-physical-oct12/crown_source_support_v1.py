"""Measure existing observed crown triangles against native animation support; no edits."""
import json,struct,hashlib,sys,io
from pathlib import Path
import numpy as np
from PIL import Image
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE));import qualify as q
from operational_records import read_record
ROOT=q.ROOT;DEST=q.OUT/'crown-source-support-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 assert not DEST.exists();DEST.mkdir()
 routes=json.loads((ROOT/'.worktrees/native-editor-delivery/work/active/croisement02/native-editor-operational-packets-v3/static/input-routes.json').read_text());path=Path(routes['mission-states/croisement02/reviewed-approved-loops-v1/canopy05/model.glb']);raw=path.read_bytes();assert sha(path)=='e576f5a00618c5beb74f13bf3a3b485cfd4ce9aa3b0f8fdfc3c59c2f0c8a83e3';size=struct.unpack_from('<I',raw,12)[0];doc=json.loads(raw[20:20+size]);buf=raw[28+size:]
 def acc(i):
  a=doc['accessors'][i];v=doc['bufferViews'][a['bufferView']];dtype={5126:'<f4',5125:'<u4',5123:'<u2',5121:'u1'}[a['componentType']];k={'SCALAR':1,'VEC2':2,'VEC3':3,'VEC4':4}[a['type']];assert 'byteStride' not in v;return np.frombuffer(buf,dtype=dtype,count=a['count']*k,offset=v.get('byteOffset',0)+a.get('byteOffset',0)).reshape(-1,k)
 native=read_record(q.OUT/'profile02-native-loop-composition-v1/report.json')['loop_frames'];rows=[]
 for node in doc['nodes']:
  if '/ Crown' not in node.get('name','') or not any(f'tree-{i:02}' in node['name'] for i in [0,4]):continue
  for pi,prim in enumerate(doc['meshes'][node['mesh']]['primitives']):
   mat=doc['materials'][prim['material']]
   if 'observed patches' not in mat['name']:continue
   tex=doc['textures'][mat['pbrMetallicRoughness']['baseColorTexture']['index']];img=doc['images'][tex['source']];view=doc['bufferViews'][img['bufferView']];atlas=np.array(Image.open(io.BytesIO(buf[view.get('byteOffset',0):view.get('byteOffset',0)+view['byteLength']])).convert('RGBA'));h,w=atlas.shape[:2]
   # All observed atlases in this group carry the same native registration.
   assert (w,h)==(756,570),(node['name'],w,h)
   masks=[]
   for frame in native:
    a=Image.new('RGBA',(w,h));a.alpha_composite(q.rgba(frame),(frame['top_left'][0]-140,frame['top_left'][1]));masks.append(np.array(a)[:,:,3]>0)
   support=np.stack(masks);uv=acc(prim['attributes']['TEXCOORD_0']);tri=acc(prim['indices']).reshape(-1,3);tuv=uv[tri];center=tuv.mean(1)
   samples=np.stack([center,*[center*.5+tuv[:,i]*.5 for i in range(3)]],axis=1);x=np.clip((samples[:,:,0]*w).astype(int),0,w-1);y=np.clip((samples[:,:,1]*h).astype(int),0,h-1);hit=support[:,y,x]
   all_never=~hit.any((0,2));all_supported_every=hit.all((0,2));all_supported_some=hit.all(2).any(0);mixed=~(all_never|all_supported_some)
   categories=np.full(len(tri),3,np.uint8);categories[all_supported_some]=2;categories[all_supported_every]=1;categories[all_never]=0
   np.save(DEST/f'{node["name"].split(" / ")[0]}-primitive{pi}-support.npy',categories)
   rows.append(dict(node=node['name'],primitive=pi,material=mat['name'],triangles=len(tri),no_native_support_at_all_four_samples_any_frame=int(all_never.sum()),all_four_samples_supported_at_least_one_frame=int(all_supported_some.sum()),all_four_samples_supported_every_frame=int(all_supported_every.sum()),mixed_or_partial_support=int(mixed.sum())))
 report=dict(status='CANDIDATE_SOURCE_OWNERSHIP_EVIDENCE_ONLY_NO_GEOMETRY_OR_OWNER_EDIT',model=dict(path=str(path),sha256=sha(path)),source_registration=dict(origin=[140,0],atlas_size=[756,570],basis='Previously four native-ray UV probes agree within0.001px; all vertices not yet requalified.'),rows=rows,limits=['Four interior UV samples per triangle are diagnostic, not exhaustive coverage or an automatic ownership rule.','No native-alpha trimming, triangle deletion, whole-crown reassignment or canonical mutation performed.','Static support requires additional source/native-role evidence; temporal alpha changes may be texture dither rather than separable physical leaves.'])
 (DEST/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(rows))
if __name__=='__main__':main()
