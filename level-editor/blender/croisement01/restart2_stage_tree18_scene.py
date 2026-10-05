"""Stage one approved combined tree placement while preserving other live records."""
import copy
import hashlib
import json
import math
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
R=ROOT/'level-editor/work/croisement01-refinement/restart2/tree18-integration-v1'
def sha(p): return hashlib.sha256(p.read_bytes()).hexdigest()
def read(p): return json.loads(p.read_text())
live=ROOT/'level-editor/library/scenes/croisement01.rhlos-map.json'
original=read(live);result=copy.deepcopy(original)
removed={'croisement01-group-052','croisement01-group-053'}
old=[p for p in original['placements'] if set(p['assets'])&removed]
assert len(old)==2 and {a for p in old for a in p['assets']}==removed
for placement in old:
    assert placement['transform']['rot_deg']==0
    asset=placement['assets'][0]
    descriptor=read(ROOT/'level-editor/library/3d-assets/croisement01'/asset/'asset.json')
    assert len(descriptor['parts'])==1
    assert descriptor['parts'][0]['node'] in ['building-052','building-053']
    x,y,z=descriptor['source_origin_scene'];t=placement['transform']
    assert abs(t['dx']-x)<1e-7 and abs(t['dy']+y*math.sin(math.radians(35)))<1e-7 and abs(t['dz']-z*math.cos(math.radians(35)))<1e-7
asset='croisement01-tree-18';folder=R/'assets'/asset
descriptor=read(folder/'asset.json');x,y,z=descriptor['source_origin_scene']
result['placements']=[p for p in result['placements'] if p not in old]
result['placements'].append(dict(id='tree-18-approved',transform=dict(dx=x,dy=-y*math.sin(math.radians(35)),dz=z*math.cos(math.radians(35)),rot_deg=0),assets=[asset]))
result['assetSources']=[p for p in result['assetSources'] if p['id'] not in removed]
base='3d-assets/croisement01/'+asset
result['assetSources'].append(dict(id=asset,model=base+'/model.glb',model_sha256=sha(folder/'model.glb'),descriptor=base+'/asset.json',descriptor_sha256=sha(folder/'asset.json')))
assert {k:v for k,v in result.items() if k not in ['placements','assetSources']}=={k:v for k,v in original.items() if k not in ['placements','assetSources']}
assert [p for p in result['placements'] if p['assets']!=[asset]]==[p for p in original['placements'] if p not in old]
(R/'croisement01.rhlos-map.json').write_text(json.dumps(result,indent=2)+'\n')
proof=dict(status='staged; GLB runtime validation and coordinated publication pending',live_scene_sha256=sha(live),staged_scene_sha256=sha(R/'croisement01.rhlos-map.json'),removed_placements=old,added_placement=result['placements'][-1],unchanged_other_placements=58,all_other_scene_fields_identical=True,asset_model_sha256=sha(folder/'model.glb'),asset_descriptor_sha256=sha(folder/'asset.json'),geometry_texture_approval=read(R/'export-proof.json'))
(R/'scene-splice-proof.json').write_text(json.dumps(proof,indent=2)+'\n')
print(R/'scene-splice-proof.json')
