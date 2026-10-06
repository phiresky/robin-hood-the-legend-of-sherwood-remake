"""Privately replace approved wood030 while preserving all other group005 parts."""
import copy,json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from asset_index import write_asset_index
from review_evidence import sha
R=ROOT/'level-editor/work/croisement01-refinement/restart2';LIB=ROOT/'level-editor/library'
def read(p):return json.loads(p.read_text())
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
def transform(d):
 x,y,z=d['source_origin_scene'];return dict(dx=x,dy=-y*math.sin(math.radians(35)),dz=z*math.cos(math.radians(35)),rot_deg=0)
def main():
 asset='croisement01-tree-00';residual='croisement01-group-005';source=R/'tree00-wood-integration-v1/assets'/asset;partition=R/'group005-tree00-partition-v1';case=R/'approved-tree00-wood-fill-v1'/asset
 scene=LIB/'scenes/croisement01.rhlos-map.json';original=read(scene);result=copy.deepcopy(original);d=read(source/'asset.json');old=read(partition/'tree00-metadata.json');proof=read(partition/'proof.json')
 assert sha(LIB/'3d-assets/croisement01'/residual/'model.glb')==proof['source_model_sha256']
 assert sha(LIB/'3d-assets/croisement01'/residual/'asset.json')==proof['source_descriptor_sha256']
 export=read(R/'tree00-wood-integration-v1/export-proof.json');assert export['geometry']['geometry_verified'] and export['approved_user_decision_sha256']==sha(case/'user-texture-decision.json')
 assert [p['node'] for p in d['parts']]==['building-030'];assert 'gameplay' not in d
 placement=next(p for p in result['placements'] if p['assets']==[residual]);assert placement['transform']==transform(old)
 before=placement['transform'];after=transform(d);a=old['parts'][0]['obstacle_local_game'];b=d['parts'][0]['obstacle_local_game'];assert {k:v for k,v in a.items() if k!='points'}=={k:v for k,v in b.items() if k!='points'}
 errors=[]
 for p,q in zip(a['points'],b['points'],strict=True):
  for key,axis in [('x','dx'),('y','dy'),('z_bottom','dz'),('z_top','dz')]:errors.append(abs(p[key]+before[axis]-q[key]-after[axis]))
 assert max(errors)<1e-9
 assert all(old['gameplay'][key]==[] for key in ['surfaces','projectionReceivers','movementBlockers','doors','lifts','interiors'])
 backup=R/'tree00-wood-integration-v1/original-export-asset.json';assert not backup.exists();write(backup,d);d['gameplay']=old['gameplay'];write(source/'asset.json',d)
 write(R/'tree00-wood-integration-v1/metadata-preservation.json',dict(status='PASS',world_coordinate_max_error=max(errors),source_partition_sha256=sha(partition/'proof.json'),descriptor_sha256=sha(source/'asset.json'),scope='Existing metadata and obstacle world coordinates retained; no new parity claim.'))
 sources={asset:source,residual:partition/residual};added=dict(id='tree00-wood-approved',assets=[asset],transform=after);assert not any(p['id']==added['id'] for p in original['placements']);result['placements'].append(added)
 assert result['placements'][:-1]==original['placements']
 result['assetSources']=[s for s in result['assetSources'] if s['id']!=residual]
 for identifier,folder in sources.items():
  base='3d-assets/croisement01/'+identifier;result['assetSources'].append(dict(id=identifier,model=base+'/model.glb',model_sha256=sha(folder/'model.glb'),descriptor=base+'/asset.json',descriptor_sha256=sha(folder/'asset.json')))
 assert [s for s in original['assetSources'] if s['id']!=residual]==[s for s in result['assetSources'] if s['id'] not in sources]
 assert {k:v for k,v in original.items() if k not in {'placements','assetSources'}}=={k:v for k,v in result.items() if k not in {'placements','assetSources'}}
 out=R/'tree00-integration-batch-v1';out.mkdir(exist_ok=False);(out/'assets').mkdir()
 for identifier,folder in sources.items():(out/'assets'/identifier).symlink_to(folder,target_is_directory=True)
 descriptors={identifier+'/asset.json':folder/'asset.json' for identifier,folder in sources.items()};write_asset_index(out/'assets',files=descriptors,descriptors=descriptors)
 write(out/scene.name,result);write(out/'scene-splice-proof.json',dict(status='Private staging only; root export appearance PASS, editor proof pending',live_scene_sha256=sha(scene),staged_scene_sha256=sha(out/scene.name),added_placements=[added],removed_placements=[],unchanged_other_placements=len(original['placements']),retained_group005_parts=proof['retained_parts'],partition_sha256=sha(partition/'proof.json'),assets={identifier:dict(model_sha256=sha(folder/'model.glb'),descriptor_sha256=sha(folder/'asset.json')) for identifier,folder in sources.items()},scope='Only approved building030 WOOD. Retained crown context remains unapproved and unpublished;029 and seven other native parts unchanged.'))
 print(out)
if __name__=='__main__':main()
