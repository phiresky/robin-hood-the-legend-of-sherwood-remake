"""Prove metadata/pivot export preserves reviewed Tree08 geometry and materials."""
import hashlib,json,struct
from pathlib import Path
import numpy as np
ROOT=Path(__file__).resolve().parents[3]
R=ROOT/'level-editor/work/active/croisement01/restart2'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def load(p):
 raw=p.read_bytes();n=struct.unpack_from('<I',raw,12)[0];return json.loads(raw[20:20+n]),raw[28+n:]
def accessor(d,b,i):
 a=d['accessors'][i];v=d['bufferViews'][a['bufferView']];assert 'byteStride' not in v
 return np.frombuffer(b,dtype={5126:'<f4',5125:'<u4',5123:'<u2'}[a['componentType']],count=a['count']*{'SCALAR':1,'VEC2':2,'VEC3':3,'VEC4':4}[a['type']],offset=v.get('byteOffset',0)+a.get('byteOffset',0)).reshape(a['count'],-1)
old=R/'tree08-approved-wood-export-v2/assets/croisement01-tree-08';new=R/'tree08-native33-export-v2/assets/croisement01-tree-08'
assert sha(old/'model.glb')=='02802ad1b5a91fadbd2e69222a64c314d89a015529a1230caaa00f8dbec5cbda'
assert sha(new/'model.glb')=='e0d5f19f4625888f3b2dfbff9e58ed0ce11a81e54b6f6d0a3aeec726846ffe8b'
a,x=load(old/'model.glb');b,y=load(new/'model.glb');ad=json.loads((old/'asset.json').read_text());bd=json.loads((new/'asset.json').read_text())
pa=np.array(ad['source_origin_scene']);pb=np.array(bd['source_origin_scene']);rows=[]
assert len(a['meshes'])==len(b['meshes'])==1
for p,q in zip(a['meshes'][0]['primitives'],b['meshes'][0]['primitives'],strict=True):
 assert p.keys()==q.keys() and p['attributes'].keys()==q['attributes'].keys()
 assert p['material']==q['material'] and p.get('mode')==q.get('mode')
 assert np.array_equal(accessor(a,x,p['indices']),accessor(b,y,q['indices']))
 for name in p['attributes']:
  u=accessor(a,x,p['attributes'][name]);v=accessor(b,y,q['attributes'][name]);assert u.shape==v.shape
  if name=='POSITION':
   error=float(np.max(np.abs(u.astype('float64')+pa-v.astype('float64')-pb)));assert error<0.0001
  elif name=='NORMAL':
   normal_error=float(np.max(np.abs(u-v)));assert np.array_equal(u,v)
  else:assert np.array_equal(u,v),name
 rows.append(dict(vertices=len(u),world_position_max_error=error,indices_uv_exact=True,normal_max_error=normal_error))
for key in ['materials','textures','samplers','extensionsUsed','extensionsRequired','scenes','scene']:assert a.get(key)==b.get(key),key
for u,v in zip(a['nodes'],b['nodes'],strict=True):
 for key in ['children','mesh','rotation','translation','scale','matrix']:assert u.get(key)==v.get(key)
images=[]
for u,v in zip(a['images'],b['images'],strict=True):
 av=a['bufferViews'][u['bufferView']];bv=b['bufferViews'][v['bufferView']]
 raw=x[av.get('byteOffset',0):av.get('byteOffset',0)+av['byteLength']];other=y[bv.get('byteOffset',0):bv.get('byteOffset',0)+bv['byteLength']];assert raw==other
 images.append(hashlib.sha256(raw).hexdigest())
report=dict(status='PASS_REVIEWED_GEOMETRY_AND_APPEARANCE_PRESERVED',reviewed_glb_sha256=sha(old/'model.glb'),native_glb_sha256=sha(new/'model.glb'),primitives=rows,embedded_image_bytes_exact=images,materials_samplers_exact=True,scope='Only metadata and local origin changed. Native gameplay preservation is checked by export/stage proofs; canopy excluded.')
p=new.parents[1]/'reviewed-export-equivalence.json';p.write_text(json.dumps(report,indent=2)+'\n');print(p);print(json.dumps(report))

proof=new.parents[1]/'export-proof.json';export=json.loads(proof.read_text());export['equivalence_sha256']=sha(p);export['status']='REVIEWED_EXPORT_EQUIVALENCE_PASS_RUNTIME_PENDING';proof.write_text(json.dumps(export,indent=2)+'\n')
