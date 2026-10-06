"""Prepare a private approved three-stump splice with exact unrelated placement preservation."""
import copy
import json
import math
import shutil
import sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from asset_index import write_asset_index
from review_evidence import sha
R=ROOT/'level-editor/work/croisement01-refinement/restart2'
LIB=ROOT/'level-editor/library'
CASES=[
 ('stump64','croisement01-southwest-broken-stump','approved-stump64-wood-fill-v1/croisement01-southwest-broken-stump',{'building-060'}),
 ('stump69','croisement01-central-ivy-stump','approved-stump69-wood-fill-v1/croisement01-central-ivy-stump',{'building-057','building-058'}),
 ('stump67','croisement01-east-ivy-stump','approved-stump67-wood-fill-v1/croisement01-east-ivy-stump',{'building-054'}),
]
def read(path):return json.loads(path.read_text())
def transform(descriptor):
 x,y,z=descriptor['source_origin_scene']
 return dict(dx=x,dy=-y*math.sin(math.radians(35)),dz=z*math.cos(math.radians(35)),rot_deg=0)
def main():
 # Validate every exact decision and derivative before writing the private stage.
 scene=LIB/'scenes/croisement01.rhlos-map.json';original=read(scene);result=copy.deepcopy(original)
 sources={};evidence={}
 for kind,asset,folder,nodes in CASES:
  stage=R/(kind+'-integration-v2');case=R/folder
  decision=read(case/'user-texture-decision.json');export=read(stage/'export-proof.json')
  assert export['approved_user_decision_sha256']==sha(case/'user-texture-decision.json')
  assert export['geometry']['geometry_verified'] is True
  assert export['geometry']['model_sha256']==decision['model_sha256']
  metadata=read(stage/'metadata-preservation.json');assert metadata['status']=='PASS'
  source=stage/'assets'/asset;d=read(source/'asset.json')
  assert sha(source/'asset.json')==metadata['descriptor_sha256']
  assert nodes <= {p['node'] for p in d['parts']}
  if kind=='stump69':
   assert export['union_of_local_surface_positions_uv_corner_ownership_materials_and_smoothing_exact'] is True
   assert export['object_world_transforms_unchanged'] is True
  sources[asset]=source
  evidence[asset]=dict(model_sha256=sha(source/'model.glb'),descriptor_sha256=sha(source/'asset.json'),metadata_sha256=sha(stage/'metadata-preservation.json'),export_sha256=sha(stage/'export-proof.json'),approval_sha256=sha(case/'user-texture-decision.json'))
 removed_ids={'croisement01-group-054','croisement01-group-057','croisement01-group-058','croisement01-group-060'}
 removed=[p for p in original['placements'] if any(a in removed_ids for a in p['assets'])]
 assert len(removed)==4 and all(len(p['assets'])==1 for p in removed)
 for placement in removed:
  d=read(LIB/'3d-assets/croisement01'/placement['assets'][0]/'asset.json')
  assert placement['transform']==transform(d)
 result['placements']=[p for p in result['placements'] if p not in removed]
 added=[]
 for kind,asset,_,_ in CASES:
  p=dict(id=kind+'-approved',assets=[asset],transform=transform(read(sources[asset]/'asset.json')))
  assert not any(old['id']==p['id'] for old in original['placements'])
  result['placements'].append(p);added.append(p)
 replaced_ids=removed_ids
 result['assetSources']=[s for s in result['assetSources'] if s['id'] not in replaced_ids]
 for asset,source in sources.items():
  base='3d-assets/croisement01/'+asset
  result['assetSources'].append(dict(id=asset,model=base+'/model.glb',model_sha256=sha(source/'model.glb'),descriptor=base+'/asset.json',descriptor_sha256=sha(source/'asset.json')))
 assert len(result['placements'])==len(original['placements'])-1
 assert [p for p in original['placements'] if p not in removed]==[p for p in result['placements'] if p not in added]
 assert {k:v for k,v in original.items() if k not in {'placements','assetSources'}}=={k:v for k,v in result.items() if k not in {'placements','assetSources'}}
 assert [s for s in original['assetSources'] if s['id'] not in replaced_ids]==[s for s in result['assetSources'] if s['id'] not in sources]
 if shutil.disk_usage(R).free<25*1024**3:raise ValueError('Disk floor25GiB')
 out=R/'stumps64-67-69-integration-batch-v1';out.mkdir(exist_ok=False)
 for asset,source in sources.items():shutil.copytree(source,out/'assets'/asset)
 write_asset_index(out/'assets')
 (out/scene.name).write_text(json.dumps(result,indent=2)+'\n')
 receipt=dict(status='Private staging only; exported appearance, full editor proof and coordinated publication pending',live_scene_sha256=sha(scene),staged_scene_sha256=sha(out/scene.name),removed_placements=removed,added_placements=added,unchanged_other_placements=len(original['placements'])-len(removed),all_other_scene_fields_identical=True,assets=evidence,scopes={'stump64':'WOOD ONLY;1424ivy/mixed pixels excluded; native60 preserved','stump69':'WOOD ONLY;1106ivy/mixed pixels excluded; native57/58 preserved','stump67':'WOOD ONLY;1945ivy/mixed pixels excluded; native54 preserved'})
 (out/'scene-splice-proof.json').write_text(json.dumps(receipt,indent=2)+'\n')
 print(out)
if __name__=='__main__':main()
