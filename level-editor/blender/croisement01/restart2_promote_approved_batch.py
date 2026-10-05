"""Install exactly approved Croisement01 replacements with scoped rollback evidence."""
import copy
import json
import os
import shutil
import sys
import tempfile
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from review_evidence import sha
from asset_index import validate_asset_index
R=ROOT/'level-editor/work/croisement01-refinement/restart2';stage=R/'approved-integration-batch-v2';library=ROOT/'level-editor/library'
def read(p):return json.loads(p.read_text())
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
proof=read(stage/'scene-splice-proof.json');runtime=read(stage/'browser-export-proof.json')
assert runtime['status']=='PASS' and runtime['palette_insert_save_reload']
assert read(stage/'integration-self-review.json')['status']=='PASS'
full_editor=read(stage/'full-editor-private-v2/result.json')
assert full_editor['status']=='PASS' and full_editor['mapGroups']==58 and full_editor['mapParts']==92
assets=proof['assets'];assert set(assets)=={x['asset'] for x in runtime['result']}
for asset,e in assets.items():
 source=stage/'assets'/asset
 assert sha(source/'model.glb')==e['model_sha256'] and sha(source/'asset.json')==e['descriptor_sha256']
 checked=next(x for x in runtime['result'] if x['asset']==asset)
 assert checked['reference']['model_sha256']==e['model_sha256'] and checked['reference']['descriptor_sha256']==e['descriptor_sha256']
 assert read(R/e['source_stage']/'glb-review-v1/inspection/self-review.json')['status']=='PASS'
 assert not (library/'3d-assets/croisement01'/asset).exists()
scene=library/'scenes/croisement01.rhlos-map.json';index=library/'3d-assets/index.json'
assert sha(scene)==proof['live_scene_sha256'];before_scene=read(scene);before_index=read(index);index_hash=sha(index)
removed={a for p in proof['removed_placements'] for a in p['assets']}
selected=[r for r in before_scene['assetSources'] if r['id'] in removed];assert len(selected)==3
for ref in selected:
 for key in ['model','descriptor']:assert sha(library/ref[key])==ref[key+'_sha256']
updated=copy.deepcopy(before_index)
for entry in read(stage/'assets/index.json')['assets']:
 assert entry['id'] in assets and not any(e['id']==entry['id'] for e in updated['assets'])
 e=copy.deepcopy(entry)
 for key in ['model','descriptor']:e[key]='croisement01/'+e[key]
 updated['assets'].append(e)
assert updated['assets'][:-3]==before_index['assets']
backup=stage/'publication-backup-v1';backup.mkdir(exist_ok=False)
shutil.copy2(scene,backup/scene.name);shutil.copy2(index,backup/'asset-index.json')
write(backup/'receipt.json',dict(scene_sha256=sha(scene),index_sha256=index_hash,selected_references=selected,rollback='Coordinate shared-index rollback with current writer and preserve subsequent additions. Existing replaced asset payloads remain intact; restoring the scene requires comparing current hash.'))
assert sha(scene)==proof['live_scene_sha256'] and sha(index)==index_hash
for asset,e in assets.items():
 target=library/'3d-assets/croisement01'/asset;temporary=Path(tempfile.mkdtemp(prefix='.'+asset+'-',dir=target.parent))
 for file in (stage/'assets'/asset).iterdir():
  assert file.is_file();shutil.copy2(file,temporary/file.name)
 os.rename(temporary,target)
validate_asset_index(library/'3d-assets',updated)
assert sha(scene)==proof['live_scene_sha256'] and sha(index)==index_hash
index_tmp=index.with_name('.index-croisement01-approved-batch.json');write(index_tmp,updated);os.replace(index_tmp,index)
assert sha(scene)==proof['live_scene_sha256']
scene_tmp=scene.with_name('.croisement01-approved-batch.json');shutil.copy2(stage/scene.name,scene_tmp);os.replace(scene_tmp,scene)
write(stage/'publication.json',dict(status='installed; full live editor check pending',scene_sha256=sha(scene),index_sha256=sha(index),assets=assets,backup=str(backup),unchanged_other_placements=56,all_other_palette_entries_preserved=True,scope=proof['scopes']))
print(stage/'publication.json')
