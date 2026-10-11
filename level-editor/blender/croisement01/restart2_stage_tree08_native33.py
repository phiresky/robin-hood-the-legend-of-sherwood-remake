"""Stage approved Tree08 visual replacement without changing native gameplay."""
import copy,json,os,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from asset_index import write_asset_index
from review_evidence import sha
R=ROOT/'level-editor/work/croisement01-refinement/restart2';LIB=ROOT/'level-editor/library'
def read(p):return json.loads(p.read_text())
def write(p,v):p.write_text(json.dumps(v,indent=2)+'\n')
asset='croisement01-tree-08';old_id='croisement01-group-033';source=R/'tree08-native33-export-v2/assets'/asset
proof=read(R/'tree08-native33-export-v2/export-proof.json');oldpath=LIB/'3d-assets/croisement01'/old_id/'asset.json'
assert sha(oldpath)==proof['old_descriptor_sha256']
old=read(oldpath);new=read(source/'asset.json')
assert new['parts']==old['parts'] and new['gameplay']==old['gameplay'] and new['source_origin_scene']==old['source_origin_scene']
scene=LIB/'scenes/croisement01.rhlos-map.json';original=read(scene);staged=copy.deepcopy(original)
placements=[p for p in staged['placements'] if old_id in p['assets']];assert len(placements)==1 and placements[0]['assets']==[old_id]
placement=placements[0];placement['assets']=[asset]
source_refs=[s for s in original['assetSources'] if s['id']==old_id];assert len(source_refs)==1
old_ref=source_refs[0]
for name in ['model','descriptor']:assert sha(LIB/old_ref[name])==old_ref[name+'_sha256']
assert not any(s['id']==asset for s in original['assetSources'])
base='3d-assets/croisement01/'+asset
new_ref=dict(id=asset,model=base+'/model.glb',model_sha256=sha(source/'model.glb'),descriptor=base+'/asset.json',descriptor_sha256=sha(source/'asset.json'))
staged['assetSources']=[new_ref if row['id']==old_id else row for row in staged['assetSources']]
assert {k:v for k,v in original.items() if k not in {'assetSources','placements'}}=={k:v for k,v in staged.items() if k not in {'assetSources','placements'}}
for a,b in zip(original['placements'],staged['placements'],strict=True):
 if a['id']==placement['id']:assert {k:v for k,v in a.items() if k!='assets'}=={k:v for k,v in b.items() if k!='assets'}
 else:assert a==b
out=R/'tree08-native33-integration-stage-v2';out.mkdir();folder=out/'assets'/asset;folder.mkdir(parents=True)
for name in ['model.glb','asset.json']:os.link(source/name,folder/name)
paths={asset+'/asset.json':folder/'asset.json'};write_asset_index(out/'assets',files=paths,descriptors=paths)
write(out/scene.name,staged)
write(out/'stage-proof.json',dict(status='PRIVATE_STAGE_REQUIRES_EXPORT_AND_EDITOR_CHECK',scope='Existing033 visual replaced by approved Tree08 wood; exact native gameplay/obstacle data, placement and unrelated map fields retained',source_map_sha256=sha(scene),staged_map_sha256=sha(out/scene.name),old_source=old_ref,new_source=new_ref,placement_id=placement['id'],placement_transform_unchanged=True,parts_and_gameplay_exact=True,source_correspondence_report_sha256=sha(R/'tree08-proxy-source-comparison-v1/report.json'),export_proof_sha256=sha(R/'tree08-native33-export-v2/export-proof.json'),approval_sha256=proof['approval_sha256'],canonical_writes=False))
print(out)
