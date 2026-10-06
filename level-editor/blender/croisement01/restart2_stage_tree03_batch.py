"""Privately stage approved Tree03 and exact native017 paint transfer."""
import copy,json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from asset_index import write_asset_index
from review_evidence import sha
R=ROOT/'level-editor/work/croisement01-refinement/restart2';LIB=ROOT/'level-editor/library'
def read(p):return json.loads(p.read_text())
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
asset='croisement01-tree-03';residual='croisement01-group-005';case=R/'approved-tree03-fill-v1'/asset;transfer=R/'tree03-visual-transfer-v1';proof=read(transfer/'proof.json');old=LIB/'3d-assets/croisement01'/residual
assert sha(old/'model.glb')==proof['source_model_sha256'] and sha(old/'asset.json')==proof['source_descriptor_sha256']
assert proof['source_pixels']==46 and proof['geometry_buffers_exact'] and proof['gameplay_and_part_metadata_exact'] and proof['outside_domain_rgba_exact']
export=read(R/'tree03-integration-v2/export-proof.json');assert export['geometry']['geometry_verified'] and export['approved_user_decision_sha256']==sha(case/'user-texture-decision.json')
source=R/'tree03-integration-v2/assets'/asset;d=read(source/'asset.json');assert {p['node'] for p in d['parts']}=={'scenery-tree03-wood','foliage-tree03-inferred-crown'};assert not any('obstacle_local_game' in p for p in d['parts'])
scene=LIB/'scenes/croisement01.rhlos-map.json';original=read(scene);result=copy.deepcopy(original);x,y,z=d['source_origin_scene'];placement=dict(id='tree03-approved',assets=[asset],transform=dict(dx=x,dy=-y*math.sin(math.radians(35)),dz=z*math.cos(math.radians(35)),rot_deg=0));assert not any(p['id']==placement['id'] for p in result['placements']);result['placements'].append(placement)
sources={asset:source,residual:transfer/'3d-assets/croisement01'/residual};result['assetSources']=[x for x in result['assetSources'] if x['id']!=residual]
for identifier,folder in sources.items():
 base='3d-assets/croisement01/'+identifier;result['assetSources'].append(dict(id=identifier,model=base+'/model.glb',model_sha256=sha(folder/'model.glb'),descriptor=base+'/asset.json',descriptor_sha256=sha(folder/'asset.json')))
assert result['placements'][:-1]==original['placements'];assert [s for s in original['assetSources'] if s['id']!=residual]==[s for s in result['assetSources'] if s['id'] not in sources];assert {k:v for k,v in original.items() if k not in ['placements','assetSources']}=={k:v for k,v in result.items() if k not in ['placements','assetSources']}
out=R/'tree03-integration-batch-v1';out.mkdir(exist_ok=False);(out/'assets').mkdir()
for identifier,folder in sources.items():(out/'assets'/identifier).symlink_to(folder,target_is_directory=True)
descriptors={identifier+'/asset.json':folder/'asset.json' for identifier,folder in sources.items()};write_asset_index(out/'assets',files=descriptors,descriptors=descriptors);write(out/scene.name,result);write(out/'scene-splice-proof.json',dict(status='Private staging only; full editor proof pending',live_scene_sha256=sha(scene),staged_scene_sha256=sha(out/scene.name),added_placements=[placement],removed_placements=[],unchanged_other_placements=len(original['placements']),transfer_proof_sha256=sha(transfer/'proof.json'),rendered_transfer_evidence_sha256=sha(transfer/'rendered-proof-v1/evidence.json'),assets={i:dict(model_sha256=sha(f/'model.glb'),descriptor_sha256=sha(f/'asset.json')) for i,f in sources.items()},scope='Approved decorativeTree03 plus only46source-cell alpha transfer from native017. All native017geometry/collisionmetadata and otherparts/placements retained exact.'))
print(out)
