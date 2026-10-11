"""Read-only inventory of untextured neutral tree-wood shader regions."""
import hashlib,json,struct
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
LIB=ROOT/'level-editor/library'
OUT=ROOT/'level-editor/work/croisement02-refinement/tree-gray-material-audit-oct12-v1'
def sha(p):
 h=hashlib.sha256()
 with p.open('rb')as f:
  while data:=f.read(1024*1024):h.update(data)
 return h.hexdigest()
def canonical_hash(x):return hashlib.sha256(json.dumps(x,sort_keys=True,separators=(',',':')).encode()).hexdigest()
def main():
 OUT.mkdir(exist_ok=True)
 scene=json.loads((LIB/'scenes/croisement02.rhlos-map.json').read_text());pins={r['id']:r for r in scene['assetSources']};rows=[]
 for folder in sorted((LIB/'3d-assets/croisement02').glob('croisement02-tree-*')):
  path=folder/'model.glb'
  with path.open('rb')as f:
   header=f.read(20);assert struct.unpack_from('<I',header,0)[0]==0x46546c67
   g=json.loads(f.read(struct.unpack_from('<I',header,12)[0]))
  matches=[];textured_wood_primitives=0
  for node in g.get('nodes',[]):
   if 'mesh'not in node:continue
   name=node.get('name','');extra=node.get('extras',{});component=str(extra.get('projection_component','')).lower()
   if 'crown'in name.lower()or'foliage'in name.lower()or component=='crown':continue
   wood=any(k in name.lower()for k in ('wood','trunk','bark','root','branch'))
   if not wood:continue
   mesh=g['meshes'][node['mesh']]
   for q in mesh['primitives']:
    if q.get('mode',4)!=4:continue
    material=g.get('materials',[])[q['material']]if'material'in q else {}
    pbr=material.get('pbrMetallicRoughness',{})
    textures=[v for k,v in material.items()if k.endswith('Texture')]+[v for k,v in pbr.items()if k.endswith('Texture')]
    if textures:textured_wood_primitives+=1;continue
    base=pbr.get('baseColorFactor',[1,1,1,1]);emissive=material.get('emissiveFactor',[0,0,0])
    neutral=lambda v:max(v[:3])-min(v[:3])<1e-6
    if not(neutral(base)and neutral(emissive)):continue
    count=g['accessors'][q['indices']]['count']if'indices'in q else g['accessors'][q['attributes']['POSITION']]['count']
    exact18=all(abs(v-.18)<1e-6 for v in emissive)and base[:3]==[0,0,0]
    matches.append({'node':name,'source_node':extra.get('source_node'),'mesh_index':node['mesh'],'material_index':q.get('material'),'material_name':material.get('name'),'triangles':count//3,'base_color':base,'emissive':emissive,'category':'constant_0.18_shader_fallback_no_source_image'if exact18 else'other_neutral_untextured_material_requires_provenance_review','material_sha256':canonical_hash(material),'primitive_sha256':canonical_hash(q),'source_geometry_tag':extra.get('reprojection_geometry_sha256'),'known_texels_tag':extra.get('reprojection_known_texels'),'unknown_texels_tag':extra.get('reprojection_unknown_texels')})
  digest=sha(path);expected=pins.get(folder.name,{}).get('model_sha256')
  if expected:assert expected==digest,(folder.name,expected,digest)
  rows.append({'asset_id':folder.name,'model':str(path),'model_sha256':digest,'map_model_pin_matches':expected==digest if expected else None,'geometry_layout_sha256':canonical_hash({k:g.get(k)for k in ('nodes','meshes','accessors','bufferViews','buffers')}),'constant_gray_regions':matches,'constant_gray_triangles':sum(r['triangles']for r in matches),'textured_wood_primitives_not_classified_by_rgb':textured_wood_primitives})
 report={'status':'READ_ONLY_SHADER_INVENTORY','map_sha256':sha(LIB/'scenes/croisement02.rhlos-map.json'),'assets':rows,'trees_inspected':len(rows),'trees_with_constant_gray_regions':sum(bool(r['constant_gray_regions'])for r in rows),'neutral_fallback_triangles':sum(r['constant_gray_triangles']for r in rows),'classification':'Uses absent material texture slots plus neutral shader constants on named wood geometry. Never classifies sampled source gray/green pixels by RGB. Constant0.18 matches the explicit unknown fallback in verified Tree39 export; other neutral constants require independent provenance. Visibility, physical role, and approval limitations are not inferred from triangle counts.','canonical_writes':False}
 (OUT/'inventory.json').write_text(json.dumps(report,indent=2)+'\n')
 print(json.dumps({k:v for k,v in report.items()if k!='assets'}));print(json.dumps([{'id':r['asset_id'],'regions':len(r['constant_gray_regions']),'triangles':r['constant_gray_triangles']}for r in rows if r['constant_gray_regions']]))
if __name__=='__main__':main()
