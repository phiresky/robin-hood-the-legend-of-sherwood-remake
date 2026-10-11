"""Stage approved Tree02 and exact retained group005 on the current saved C1 map."""
import copy,hashlib,json,math,os,struct,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];R=ROOT/'level-editor/work/croisement01-refinement/restart2';LIB=ROOT/'level-editor/library'
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from asset_index import write_asset_index
O=R/'tree02-integration-batch-v1';assert not O.exists()
TREE='croisement01-tree-02';GROUP='croisement01-group-005';removed={'building-017','building-025'}
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def read(p):return json.loads(p.read_text())
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
def transform(d):
 x,y,z=d['source_origin_scene'];return dict(dx=x,dy=-y*math.sin(math.radians(35)),dz=z*math.cos(math.radians(35)),rot_deg=0)
export=R/'tree02-approved-integration-v1';handoff=read(export/'handoff.json');assert handoff['status']=='READY_FOR_PRIVATE_MAP_STAGE_AND_BROWSER'
source=export/'assets'/TREE;assert sha(source/'model.glb')==handoff['model_sha256']=='48676ff322a34c44c6c258402a9b44587b1d7a29bbd1570ead4adeff9e3f7751'
assert sha(Path(handoff['self_review']))==handoff['self_review_sha256']
scene=LIB/'scenes/croisement01.rhlos-map.json';before=read(scene);scene_sha=sha(scene);assert any(x['id']=='croisement01-tree-01' for x in before['assetSources']);assert not any(x['id']==TREE for x in before['assetSources'])
ref=next(x for x in before['assetSources'] if x['id']==GROUP);oldfolder=LIB/Path(ref['descriptor']).parent
assert sha(oldfolder/'model.glb')==ref['model_sha256'] and sha(oldfolder/'asset.json')==ref['descriptor_sha256']
old=read(oldfolder/'asset.json');new=read(source/'asset.json');assert {p['node'] for p in new['parts']}==removed|{'foliage-tree02-inferred-crown'};assert 'gameplay' not in new
allparts={p['node'] for p in old['parts']};assert allparts=={'building-005','building-010','building-014','building-015','building-016','building-017','building-025'};kept=allparts-removed
placement=next(p for p in before['placements'] if p['assets']==[GROUP]);assert placement['transform']==transform(old)
errors=[]
for node in sorted(removed):
 a=next(p for p in old['parts'] if p['node']==node);b=next(p for p in new['parts'] if p['node']==node)
 assert a['source_obstacle']==b['source_obstacle'];ao=a['obstacle_local_game'];bo=b['obstacle_local_game'];assert {k:v for k,v in ao.items() if k!='points'}=={k:v for k,v in bo.items() if k!='points'}
 for p,q in zip(ao['points'],bo['points'],strict=True):
  for key,axis in [('x','dx'),('y','dy'),('z_bottom','dz'),('z_top','dz')]:errors.append(abs(p[key]+transform(old)[axis]-q[key]-transform(new)[axis]))
assert max(errors)<1e-9
metadata=old['gameplay'];assert set(metadata)=={'version','collision','surfaces','projectionReceivers','sightOrder','movementBlockers','doors','lifts','interiors','draft'}
partition={}
for label,domain in [('retained',kept),('tree',removed)]:
 item=copy.deepcopy(metadata)
 for key in ['surfaces','projectionReceivers','movementBlockers','doors','lifts','interiors']:
  assert all(row.get('node') in allparts for row in metadata[key]);item[key]=[copy.deepcopy(row) for row in metadata[key] if row['node'] in domain]
 item['sightOrder']={k:v for k,v in metadata['sightOrder'].items() if k in domain};partition[label]=item
for key in ['surfaces','projectionReceivers','movementBlockers','doors','lifts','interiors']:
 assert sorted(partition['tree'][key]+partition['retained'][key],key=lambda x:x['id'])==sorted(metadata[key],key=lambda x:x['id'])
assert {**partition['tree']['sightOrder'],**partition['retained']['sightOrder']}==metadata['sightOrder']
raw=(oldfolder/'model.glb').read_bytes();magic,version,total=struct.unpack_from('<III',raw);assert magic==0x46546c67 and version==2 and total==len(raw);size,kind=struct.unpack_from('<II',raw,12);assert kind==0x4e4f534a
gltf=read_original=json.loads(raw[20:20+size]);gltf=copy.deepcopy(gltf);indices={i for i,n in enumerate(gltf['nodes']) if n.get('name') in removed};assert len(indices)==2 and not gltf.get('animations') and not gltf.get('skins')
mapping={old_i:new_i for new_i,old_i in enumerate(i for i in range(len(gltf['nodes'])) if i not in indices)};nodes=[]
for i,node in enumerate(gltf['nodes']):
 if i in indices:continue
 if 'children' in node:node['children']=[mapping[c] for c in node['children'] if c not in indices]
 nodes.append(node)
gltf['nodes']=nodes
for item in gltf['scenes']:item['nodes']=[mapping[n] for n in item['nodes'] if n not in indices]
assert {k:v for k,v in gltf.items() if k not in ['nodes','scenes']}=={k:v for k,v in read_original.items() if k not in ['nodes','scenes']}
for node in kept:assert next(n for n in gltf['nodes'] if n.get('name')==node)==next(n for n in read_original['nodes'] if n.get('name')==node)
resources={x['path']:x['sha256'] for x in old['resources']}
for path,digest in resources.items():assert sha(LIB/path)==digest
O.mkdir();(O/'assets').mkdir();td=O/'assets'/TREE;gd=O/'assets'/GROUP;td.mkdir();gd.mkdir();os.link(source/'model.glb',td/'model.glb');new['gameplay']=partition['tree'];write(td/'asset.json',new)
retained=copy.deepcopy(old);retained['parts']=[p for p in old['parts'] if p['node'] in kept];retained['gameplay']=partition['retained'];write(gd/'asset.json',retained)
encoded=json.dumps(gltf,separators=(',',':')).encode();encoded+=b' '*((-len(encoded))%4);tail=raw[20+size:];(gd/'model.glb').write_bytes(struct.pack('<III',magic,version,20+len(encoded)+len(tail))+struct.pack('<II',len(encoded),kind)+encoded+tail)
result=copy.deepcopy(before);newref=next(x for x in result['assetSources'] if x['id']==GROUP);newref['model_sha256']=sha(gd/'model.glb');newref['descriptor_sha256']=sha(gd/'asset.json')
base='3d-assets/croisement01/'+TREE;result['assetSources'].append(dict(id=TREE,model=base+'/model.glb',model_sha256=sha(td/'model.glb'),descriptor=base+'/asset.json',descriptor_sha256=sha(td/'asset.json')))
added=dict(id='tree02-approved',assets=[TREE],transform=transform(new));assert not any(p['id']==added['id'] for p in before['placements']);result['placements'].append(added)
assert result['placements'][:-1]==before['placements'];assert {k:v for k,v in result.items() if k not in ['placements','assetSources']}=={k:v for k,v in before.items() if k not in ['placements','assetSources']}
for oldref in before['assetSources']:
 if oldref['id']!=GROUP:assert next(x for x in result['assetSources'] if x['id']==oldref['id'])==oldref
write(O/scene.name,result);write(O/'baseline-map.json',before)
assets={aid:dict(model_sha256=sha(O/'assets'/aid/'model.glb'),descriptor_sha256=sha(O/'assets'/aid/'asset.json')) for aid in [TREE,GROUP]}
files={aid+'/asset.json':O/'assets'/aid/'asset.json' for aid in assets};write_asset_index(O/'assets',files=files,descriptors=files)
assert sha(scene)==scene_sha and sha(source/'model.glb')==handoff['model_sha256']
write(O/'scene-splice-proof.json',dict(status='PRIVATE_STAGE_REQUIRES_CATALOG_AND_BROWSER',live_scene_sha256=scene_sha,staged_scene_sha256=sha(O/scene.name),approved_export_handoff_sha256=sha(export/'handoff.json'),approved_export_descriptor_sha256=sha(source/'asset.json'),source_group_model_sha256=ref['model_sha256'],source_group_descriptor_sha256=ref['descriptor_sha256'],added_placements=[added],unchanged_other_placements=len(before['placements']),retained_parts=sorted(kept),removed_native_parts=sorted(removed),new_scenery_parts=['foliage-tree02-inferred-crown'],retained_mesh_nodes_exact=True,all_non_node_scene_gltf_fields_exact=True,unchanged_trailing_chunks_sha256=hashlib.sha256(tail).hexdigest(),world_obstacle_max_error=max(errors),gameplay_partition_lossless=True,shared_resources=resources,assets=assets,canonical_writes=False))
print(O)
