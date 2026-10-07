"""Read-only installed canopy material/vertex provenance for path conflicts."""
from pathlib import Path
import hashlib,json,struct,mmap
import numpy as np
ROOT=Path(__file__).resolve().parents[3];LIB=ROOT/'level-editor/library';BASE=ROOT/'level-editor/work/croisement02-refinement/restart14-canopy-animation';DEST=BASE/'butterfly-canopy-provenance-v1'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def inspect(aid,src):
 path=LIB/src['model'];raw=path.read_bytes();n=struct.unpack_from('<I',raw,12)[0];doc=json.loads(raw[20:20+n]);offset=28+n;external={};resources=[]
 def acc(i):
  a=doc['accessors'][i];v=doc['bufferViews'][a['bufferView']];dt=np.dtype({5126:'<f4',5125:'<u4',5123:'<u2',5121:'u1'}[a['componentType']]);width={'SCALAR':1,'VEC2':2,'VEC3':3,'VEC4':4}[a['type']];bi=v.get('buffer',0)
  if bi and bi not in external:
   p=path.parent/doc['buffers'][bi]['uri'];external[bi]=p.read_bytes();resources.append({'path':str(p),'sha256':sha(p)})
  arr=np.ndarray((a['count'],width),dtype=dt,buffer=external[bi]if bi else raw,offset=(0 if bi else offset)+v.get('byteOffset',0)+a.get('byteOffset',0),strides=(v.get('byteStride',dt.itemsize*width),dt.itemsize)).copy()
  if a.get('normalized'):arr=arr.astype(float)/np.iinfo(dt).max
  return arr
 rows=[]
 for ni,node in enumerate(doc['nodes']):
  if 'mesh'not in node or node.get('extras',{}).get('projection_component')!='crown':continue
  for pi,pr in enumerate(doc['meshes'][node['mesh']]['primitives']):
   mi=pr['material'];m=doc['materials'][mi];attrs=pr['attributes'];color=acc(attrs['COLOR_0']) if 'COLOR_0'in attrs else None;tex=m['pbrMetallicRoughness']['baseColorTexture'];td=doc['textures'][tex['index']];im=doc['images'][td['source']];v=doc['bufferViews'][im['bufferView']];payload=raw[offset+v.get('byteOffset',0):offset+v.get('byteOffset',0)+v['byteLength']];uv=acc(attrs[f"TEXCOORD_{tex.get('texCoord',0)}"])
   rows.append({'node_index':ni,'node':node['name'],'mesh_index':node['mesh'],'primitive_index':pi,'material_index':mi,'material':m,'image':{'index':td['source'],'name':im.get('name'),'mimeType':im.get('mimeType'),'encoded_sha256':hashlib.sha256(payload).hexdigest()},'sampler':doc.get('samplers',[])[td['sampler']]if'sampler'in td else {},'vertex_count':len(uv),'triangle_count':doc['accessors'][pr['indices']]['count']//3,'ownership_color_r_range':None if color is None else [float(color[:,0].min()),float(color[:,0].max())],'vertex_alpha_range':[1.,1.]if color is None or color.shape[1]<4 else [float(color[:,3].min()),float(color[:,3].max())],'uv_range':[uv.min(0).tolist(),uv.max(0).tolist()]})
 return {'asset':aid,'model':str(path),'model_sha256':sha(path),'descriptor':str(LIB/src['descriptor']),'descriptor_sha256':sha(LIB/src['descriptor']),'resources':resources,'crown_primitives':rows}
def main():
 mp=LIB/'scenes/croisement02.rhlos-map.json';m=json.loads(mp.read_text());ids=['croisement02-tree-42','croisement02-tree-03'];records=[inspect(a,next(s for s in m['assetSources']if s['id']==a))for a in ids];report={'status':'READ_ONLY_MATERIAL_REGISTRY_AWAITING_EXACT_VALID_HITS','map_sha256':sha(mp),'assets':records,'limitations':['Material source-ownership red channel and physical opacity alpha are separate.','Observed-front texture provenance does not independently prove inferred3D leaf depth or source-specific foreground priority.','Embedded candidate-review-pending strings are historical generation metadata; they do not supersede archived user/model approvals.','No model/texture/runtime changes and no Blender or rendering.']};DEST.mkdir(exist_ok=False);(DEST/'material-registry.json').write_text(json.dumps(report,indent=2)+'\n');assert sum(p.stat().st_size for p in DEST.rglob('*')if p.is_file())<2*2**20
 print([(r['asset'],[(p['material_index'],p['ownership_color_r_range'],p['vertex_alpha_range'])for p in r['crown_primitives']])for r in records])
if __name__=='__main__':main()
