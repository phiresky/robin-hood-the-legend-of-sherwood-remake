"""Bundle complete native canopy groups without changing their physical mesh data."""
import argparse,copy,json,struct,hashlib,math,shutil
from pathlib import Path
from forest_layout import CROWNS
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()

def main():
 p=argparse.ArgumentParser();p.add_argument('--exports',type=Path,nargs='+',required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();a.output.mkdir(exist_ok=False);reports=[json.loads(ep.read_text()) for ep in a.exports];indices={r['animation_index'] for r in reports};assert len(indices)==1;index=indices.pop();assets=[asset for r in reports for asset in r['assets']];expected={f'croisement02-tree-{x[0]:02}' for x in CROWNS[index]};assert {x['asset_id'] for x in assets}==expected and len(assets)==len(expected),'Incomplete native canopy ownership group'
 total=sum(Path(x['model']['path']).stat().st_size for x in assets);assert shutil.disk_usage(a.output).free>=total+10*1024**3
 arrays=['bufferViews','accessors','images','samplers','textures','materials','meshes','nodes'];doc=dict(asset=dict(version='2.0',generator='Private physical canopy group bundler'),scene=0,scenes=[dict(name=f'Canopy group {index:02}',nodes=[])],buffers=[],animations=[dict(name=f'Canopy group {index:02} inferred motion',channels=[],samplers=[])],extensionsUsed=['KHR_materials_unlit']);doc.update({key:[] for key in arrays});binary=bytearray();records=[]
 for asset in assets:
  path=Path(asset['model']['path']);assert sha(path)==asset['model']['sha256'];raw=path.read_bytes();n=struct.unpack_from('<I',raw,12)[0];src=json.loads(raw[20:20+n]);assert set(src.get('extensionsUsed',[]))<= {'KHR_materials_unlit'} and not src.get('skins');assert len(src['buffers'])==1 and all('uri' not in x for x in src['images']);data=raw[28+n:28+n+src['buffers'][0]['byteLength']];binary.extend(b'\0'*(-len(binary)%4));offset=len(binary);binary.extend(data);base={key:len(doc[key]) for key in arrays};samplerbase=len(doc['animations'][0]['samplers']);nodes=copy.deepcopy(src['nodes']);parts={key:copy.deepcopy(src.get(key,[])) for key in arrays}
  for v in parts['bufferViews']:assert v.get('buffer',0)==0;v['buffer']=0;v['byteOffset']=v.get('byteOffset',0)+offset
  for x in parts['accessors']:
   assert 'sparse' not in x
   if 'bufferView' in x:x['bufferView']+=base['bufferViews']
  for x in parts['images']:x['bufferView']+=base['bufferViews']
  for x in parts['textures']:
   if 'source' in x:x['source']+=base['images']
   if 'sampler' in x:x['sampler']+=base['samplers']
  def textures(value):
   if isinstance(value,dict):
    for key,item in value.items():
     if key.endswith('Texture') and isinstance(item,dict) and 'index' in item:item['index']+=base['textures']
     else:textures(item)
   elif isinstance(value,list):
    for item in value:textures(item)
  for x in parts['materials']:textures(x)
  for mesh in parts['meshes']:
   for primitive in mesh['primitives']:
    for key in primitive['attributes']:primitive['attributes'][key]+=base['accessors']
    if 'indices' in primitive:primitive['indices']+=base['accessors']
    if 'material' in primitive:primitive['material']+=base['materials']
    for target in primitive.get('targets',[]):
     for key in target:target[key]+=base['accessors']
  for node in parts['nodes']:
   if 'mesh' in node:node['mesh']+=base['meshes']
   if 'children' in node:node['children']=[i+base['nodes'] for i in node['children']]
   if 'name' in node:node['name']=asset['asset_id']+' / '+node['name']
  for key in arrays:doc[key].extend(parts[key])
  animation=copy.deepcopy(src['animations'][0]);assert len(src['animations'])==1
  for sampler in animation['samplers']:sampler['input']+=base['accessors'];sampler['output']+=base['accessors']
  for channel in animation['channels']:channel['sampler']+=samplerbase;channel['target']['node']+=base['nodes']
  doc['animations'][0]['samplers'].extend(animation['samplers']);doc['animations'][0]['channels'].extend(animation['channels']);transform=asset['placement']['transform'];assert transform['rot_deg']==0;root=len(doc['nodes']);doc['nodes'].append(dict(name=asset['asset_id']+' placed root',translation=[transform['dx'],transform['dz'],transform['dy']/math.sin(math.radians(35))],children=[i+base['nodes'] for i in src['scenes'][src.get('scene',0)]['nodes']]));doc['scenes'][0]['nodes'].append(root)
  assert bytes(binary[offset:offset+len(data)])==data;records.append(dict(asset_id=asset['asset_id'],source_model=asset['model'],source_binary_sha256=hashlib.sha256(data).hexdigest(),binary_offset=offset,binary_bytes=len(data),placed_root=root,node_range=[base['nodes'],root],placement=asset['placement'],static_source=asset['source_model'],physical_basis_and_morph_bytes_exact=True))
 doc['buffers']=[dict(byteLength=len(binary))];binary.extend(b'\0'*(-len(binary)%4));encoded=json.dumps(doc,separators=(',',':')).encode();encoded+=b' '*(-len(encoded)%4);model=a.output/f'canopy-{index:02}-loop.glb';model.write_bytes(struct.pack('<III',0x46546c67,2,28+len(encoded)+len(binary))+struct.pack('<II',len(encoded),0x4e4f534a)+encoded+struct.pack('<II',len(binary),0x004e4942)+binary)
 report=dict(status='PRIVATE_COMPLETE_NATIVE_GROUP_REQUIRES_COMBINED_IMPORT_REVIEW',animation_index=index,model=dict(path=str(model),sha256=sha(model)),exports=[dict(path=str(ep),sha256=sha(ep)) for ep in a.exports],assets=records,clip=doc['animations'][0]['name'],cycle_ticks=56,scope='Complete native group, absolute editorYup placement, one synchronized clip. Node names prefixed for unambiguous loading; exact geometry/morph/texture bytes retained.',limits=['All inferred motion remains unapproved.','Combined loader/context and static replacement lifecycle pending.','No canonical assets or runtime edits.']);(a.output/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(index,len(records),sha(model))
if __name__=='__main__':main()
