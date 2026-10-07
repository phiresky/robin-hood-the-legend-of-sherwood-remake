"""Verify the private GLB retains receiver artwork, geometry and UV interpolation."""
import json,struct,hashlib
from pathlib import Path
import numpy as np
ROOT=Path(__file__).resolve().parents[2]/'work/croisement02-refinement/restart10-hole-aperture'
f=ROOT/'export-v1/receivers-and-endpoints.glb';raw=f.read_bytes();size=struct.unpack_from('<I',raw,12)[0];doc=json.loads(raw[20:20+size]);blob=raw[28+size:]
packet=json.loads((ROOT/'packet-v1/packet.json').read_text());phase=json.loads((ROOT.parent/'restart9-hole-endpoints/source-plan-v1/plan.json').read_text())
known={i['packed_sha256']for m in packet['meshes']for mat in m['materials']for i in mat['images']if i['packed_sha256']}|{p['sha256']for p in phase['phases']}
images=[]
for image in doc['images']:
 view=doc['bufferViews'][image['bufferView']];data=blob[view.get('byteOffset',0):view.get('byteOffset',0)+view['byteLength']];digest=hashlib.sha256(data).hexdigest();assert digest in known,(image['name'],digest);images.append({'name':image['name'],'sha256':digest,'source_bytes_exact':True})
def accessor(index):
 a=doc['accessors'][index];v=doc['bufferViews'][a['bufferView']];dtype={5126:'<f4',5125:'<u4',5123:'<u2',5121:'u1'}[a['componentType']];count={'SCALAR':1,'VEC2':2,'VEC3':3,'VEC4':4}[a['type']];step=np.dtype(dtype).itemsize;offset=v.get('byteOffset',0)+a.get('byteOffset',0)
 return np.ndarray((a['count'],count),dtype=dtype,buffer=blob,offset=offset,strides=(v.get('byteStride',step*count),step))
rows=[];max_position=max_uv=0.;tested_triangles=0
for record in packet['meshes']:
 original=next(n for n in doc['nodes']if n.get('name')==record['name'])
 if len(record['groups'])==1:
  copied=next(n for n in doc['nodes']if n.get('name')==record['name']+' / outside')
  assert original['mesh']==copied['mesh'];assert all(original.get(k)==copied.get(k)for k in ['matrix','translation','rotation','scale']);rows.append({'source':record['name'],'original_mesh_and_transform_shared_exact':True});continue
 for tag,triangles in record['groups'].items():
  node=next(n for n in doc['nodes']if n.get('name')==record['name']+' / '+tag);assert all(k not in node for k in ['matrix','translation','rotation','scale'])
  primitives=doc['meshes'][node['mesh']]['primitives'];count=0
  for primitive in primitives:
   material=doc['materials'][primitive['material']]['name'];mi=next(i for i,m in enumerate(record['materials'])if m['name']==material);expected=[r for r in triangles if r['material']==mi];indices=accessor(primitive['indices']).reshape(-1);assert len(indices)==3*len(expected)
   positions=accessor(primitive['attributes']['POSITION'])[indices].reshape(-1,3,3);xyz=np.array([r['xyz']for r in expected]);yup=xyz[:,:,[0,2,1]].copy();yup[:,:,2]*=-1
   drift=float(np.max(np.abs(positions-yup)));max_position=max(max_position,drift);assert drift<.001,(record['name'],tag,drift)
   for i,name in enumerate(expected[0]['uv']):
    actual=accessor(primitive['attributes'][f'TEXCOORD_{i}'])[indices].reshape(-1,3,2);uv=np.array([r['uv'][name]for r in expected]);uv[:,:,1]=1-uv[:,:,1];drift=float(np.max(np.abs(actual-uv)));max_uv=max(max_uv,drift);assert drift<2e-7,(record['name'],tag,name,drift)
   count+=len(expected)
  assert count==len(triangles);tested_triangles+=count;rows.append({'source':record['name'],'component':tag,'triangles':count,'materials_shared_exact':True})
report={'status':'PASS','model_sha256':hashlib.sha256(raw).hexdigest(),'packet_sha256':hashlib.sha256((ROOT/'packet-v1/packet.json').read_bytes()).hexdigest(),'images':images,'receiver_parts':rows,'exported_triangles_compared':tested_triangles,'max_position_serialization_drift':max_position,'max_uv_serialization_drift':max_uv,'scope':'Exact original texture bytes and shared material definitions; unchanged receiver meshes/transform references preserved; clipped receiver geometry and every UV layer agree within float32 serialization. No approval or live publication.'}
(ROOT/'export-v1/export-guards.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:v for k,v in report.items()if k not in ['images','receiver_parts']}))
