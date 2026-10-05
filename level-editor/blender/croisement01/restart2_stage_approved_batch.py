"""Stage three exactly approved replacements without modifying the live library."""
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
CASES=[('rock29-integration-v1','croisement01-small-bank-stones','018','rock-29-approved'),('tree20-integration-v2','croisement01-tree-20','082','tree-20-approved'),('stump68-integration-v2','croisement01-southeast-small-stump','059','stump-68-wood-approved')]
out=R/'approved-integration-batch-v2';out.mkdir(exist_ok=False)
library=ROOT/'level-editor/library';live=library/'scenes/croisement01.rhlos-map.json';original=json.loads(live.read_text());result=copy.deepcopy(original)
removed=[];added=[];evidence={}
for stage,asset,node,placement_id in CASES:
 source=R/stage/'assets'/asset;d=json.loads((source/'asset.json').read_text())
 assert json.loads((R/stage/'metadata-preservation.json').read_text())['status']=='PASS'
 old_id='croisement01-group-'+node;old=next(p for p in original['placements'] if p['assets']==[old_id]);assert old['transform']['rot_deg']==0
 old_d=json.loads((library/'3d-assets/croisement01'/old_id/'asset.json').read_text());x,y,z=old_d['source_origin_scene'];t=old['transform']
 assert max(abs(t['dx']-x),abs(t['dy']+y*math.sin(math.radians(35))),abs(t['dz']-z*math.cos(math.radians(35))))<1e-7
 assert any(p['node']=='building-'+node for p in d['parts'])
 x,y,z=d['source_origin_scene'];placement=dict(id=placement_id,assets=[asset],transform=dict(dx=x,dy=-y*math.sin(math.radians(35)),dz=z*math.cos(math.radians(35)),rot_deg=0))
 result['placements']=[p for p in result['placements'] if p!=old];result['placements'].append(placement)
 result['assetSources']=[p for p in result['assetSources'] if p['id']!=old_id]
 base='3d-assets/croisement01/'+asset
 result['assetSources'].append(dict(id=asset,model=base+'/model.glb',model_sha256=sha(source/'model.glb'),descriptor=base+'/asset.json',descriptor_sha256=sha(source/'asset.json')))
 shutil.copytree(source,out/'assets'/asset);removed.append(old);added.append(placement)
 evidence[asset]=dict(source_stage=stage,model_sha256=sha(source/'model.glb'),descriptor_sha256=sha(source/'asset.json'),metadata_sha256=sha(R/stage/'metadata-preservation.json'),export_sha256=sha(R/stage/'export-proof.json'))
assert len(result['placements'])==len(original['placements'])==59
assert [p for p in original['placements'] if p not in removed]==[p for p in result['placements'] if p not in added]
assert {k:v for k,v in original.items() if k not in ['placements','assetSources']}=={k:v for k,v in result.items() if k not in ['placements','assetSources']}
write_asset_index(out/'assets')
(out/live.name).write_text(json.dumps(result,indent=2)+'\n')
(out/'scene-splice-proof.json').write_text(json.dumps(dict(status='private staged candidate; runtime and coordinated promotion pending',live_scene_sha256=sha(live),staged_scene_sha256=sha(out/live.name),removed_placements=removed,added_placements=added,unchanged_other_placements=56,all_other_scene_fields_identical=True,assets=evidence,scopes={'stump68':'WOOD ONLY;786 surrounding foliage pixels remain deferred','tree20':'Approved geometry and texture; excluded neighboring branch and final scene joints remain separate','rock29':'Approved geometry and texture; inferred reverse mottling is appearance only'}),indent=2)+'\n')
print(out)
