"""Replace installed wood mesh references while retaining all crown resources verbatim."""
import copy,json,hashlib,struct,sys
from pathlib import Path
import numpy as np
R=Path.cwd();B=R/'level-editor/work/croisement02-refinement/approved-round-wood-integration-oct11-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def load(p):
 raw=p.read_bytes();n=struct.unpack_from('<I',raw,12)[0];return json.loads(raw[20:20+n]),raw[28+n:]
def pack(d,b):
 b+=b'\0'*(-len(b)%4);d['buffers'][0]['byteLength']=len(b);j=json.dumps(d,separators=(',',':')).encode();j+=b' '*(-len(j)%4);return struct.pack('<4sII',b'glTF',2,28+len(j)+len(b))+struct.pack('<II',len(j),0x4e4f534a)+j+struct.pack('<II',len(b),0x004e4942)+b

def main(n):
 base=B/f'tree-{n}-v1';report=json.loads((base/'export.json').read_text());old=Path(report['current_model']);rawpath=base/'exact/model.glb';assert all(sha(Path(p))==h for p,h in report['pins'].items());od,ob=load(old);nd,nb=load(rawpath);out=copy.deepcopy(od);body=bytearray(ob);mapping={};changed=[]
 assert len(nd['buffers'])==1 and 'uri'not in od['buffers'][0]
 def append(kind,i):
  key=(kind,i)
  if key in mapping:return mapping[key]
  row=copy.deepcopy(nd[kind][i])
  if kind=='bufferViews':
   assert row.get('buffer',0)==0 and not row.get('extensions');data=nb[row.get('byteOffset',0):row.get('byteOffset',0)+row['byteLength']];body.extend(b'\0'*(-len(body)%4));row.update(buffer=0,byteOffset=len(body));body.extend(data)
  elif kind=='accessors':
   assert 'sparse'not in row;row['bufferView']=append('bufferViews',row['bufferView'])
  elif kind=='images':
   if 'bufferView'in row:row['bufferView']=append('bufferViews',row['bufferView'])
   else:
    payload=(rawpath.parent/row.pop('uri')).read_bytes();body.extend(b'\0'*(-len(body)%4));row['bufferView']=len(out.setdefault('bufferViews',[]));out['bufferViews'].append(dict(buffer=0,byteOffset=len(body),byteLength=len(payload)));body.extend(payload);row['mimeType']='image/png'if payload.startswith(b'\x89PNG')else'image/jpeg'
  elif kind=='textures':
   assert not row.get('extensions');row['source']=append('images',row['source'])
   if 'sampler'in row:row['sampler']=append('samplers',row['sampler'])
  elif kind=='materials':
   def visit(d):
    for k,v in d.items():
     if isinstance(v,dict):
      if k.endswith('Texture')and'index'in v:v['index']=append('textures',v['index'])
      else:visit(v)
   visit(row)
  elif kind=='meshes':
   for p in row['primitives']:
    assert not p.get('extensions')and not p.get('targets');p['attributes']={k:append('accessors',v)for k,v in p['attributes'].items()}
    if'indices'in p:p['indices']=append('accessors',p['indices'])
    if'material'in p:p['material']=append('materials',p['material'])
  elif kind!='samplers':raise ValueError(kind)
  idx=len(out.setdefault(kind,[]));out[kind].append(row);mapping[key]=idx;return idx
 for owner in ([80,81,82]if n==32 else[94]):
  name=f'building-{owner:03d}';oi=next(i for i,r in enumerate(od['nodes'])if r.get('name')==name);ni=next(i for i,r in enumerate(nd['nodes'])if r.get('name')==name)
  oldchildren=[i for i in od['nodes'][oi]['children']if'mesh'in od['nodes'][i]and'crown'not in od['nodes'][i].get('name','').lower()];newchildren=[i for i in nd['nodes'][ni]['children']if'mesh'in nd['nodes'][i]and'crown'not in nd['nodes'][i].get('name','').lower()];assert len(oldchildren)==len(newchildren)==1
  a=oldchildren[0];b=newchildren[0]
  for key,default in [('matrix',None),('translation',[0,0,0]),('rotation',[0,0,0,1]),('scale',[1,1,1])]:
   assert od['nodes'][a].get(key,default)==nd['nodes'][b].get(key,default)
  out['nodes'][a]['mesh']=append('meshes',nd['nodes'][b]['mesh']);changed.append(dict(source_node=name,node_index=a,prior_mesh=od['nodes'][a]['mesh'],new_mesh=out['nodes'][a]['mesh'],export_node=nd['nodes'][b]['name']))
 # Original arrays, including crown accessors, image payloads, samplers and materials, remain exact.
 assert bytes(body[:len(ob)])==ob
 for kind in ['accessors','bufferViews','images','textures','samplers','materials','meshes']:
  assert out.get(kind,[])[:len(od.get(kind,[]))]==od.get(kind,[])
 check=copy.deepcopy(out['nodes'])
 for r in changed:check[r['node_index']]['mesh']=r['prior_mesh']
 assert check==od['nodes'] and out['scenes']==od['scenes'];folder=base/'delivery-v1';folder.mkdir(exist_ok=False);(folder/'model.glb').write_bytes(pack(out,bytes(body)));(folder/'asset.json').write_bytes(Path(report['current_descriptor']).read_bytes());final,fb=load(folder/'model.glb');assert fb[:len(ob)]==ob
 external=[]
 for record in [*od.get('buffers',[]),*od.get('images',[])]:
  if 'uri'not in record:continue
  source=(old.parent/record['uri']).resolve();target=(folder/record['uri']).resolve();assert target.is_relative_to(B.resolve());target.parent.mkdir(parents=True,exist_ok=True)
  if target.exists():assert sha(target)==sha(source)
  else:target.write_bytes(source.read_bytes())
  external.append(dict(uri=record['uri'],source=str(source),path=str(target),sha256=sha(source)))
 result=dict(status='PASS_PRIVATE_EXACT_INSTALLED_CROWN_RETENTION',model_sha256=sha(folder/'model.glb'),descriptor_sha256=sha(folder/'asset.json'),installed_model_sha256=sha(old),raw_export_sha256=sha(rawpath),changed_wood_nodes=changed,external_resources=external,original_nodes_and_transforms_exact_except_wood_mesh_references=True,all_original_resource_records_exact=True,original_binary_prefix_exact=True,crown_geometry_uv_materials_images_samplers_exact=True,source_model_unchanged=sha(Path(report['source_model']))==report['source_sha256'],scope='New approved wood meshes only; current complete crown and gameplay descriptor retained. Delivery derivative and independent surface/browser proof pending.')
 (base/'merge-proof.json').write_text(json.dumps(result,indent=2)+'\n');print(n,result['model_sha256'])
if __name__=='__main__':main(int(sys.argv[1]))
