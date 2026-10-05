"""Install the scoped approved tree after runtime proof, preserving rollback files."""
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
R=ROOT/'level-editor/work/croisement01-refinement/restart2/tree18-integration-v1'
def read(p):return json.loads(p.read_text())
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
proof=read(R/'scene-splice-proof.json');browser=read(R/'browser-export-proof.json');visual=read(R/'glb-review-v1/inspection/self-review.json')
assert browser['status']=='PASS' and browser['palette_insert_save_reload'] and visual['status']=='PASS'
assert read(R/'metadata-preservation.json')['status']=='PASS'
asset='croisement01-tree-18';source=R/'assets'/asset;library=ROOT/'level-editor/library';target=library/'3d-assets/croisement01'/asset
assert not target.exists()
assert browser['result']['reference']['descriptor_sha256']==sha(source/'asset.json')==proof['asset_descriptor_sha256']
assert browser['result']['reference']['model_sha256']==sha(source/'model.glb')==proof['asset_model_sha256']
assert read(R/'glb-review-v1/glb-proof.json')['source_glb_sha256']==sha(source/'model.glb')
scene=library/'scenes/croisement01.rhlos-map.json';index=library/'3d-assets/index.json'
assert sha(scene)==proof['live_scene_sha256']
current_scene=read(scene)
for ref in current_scene['assetSources']:
    if ref['id'] in ['croisement01-group-052','croisement01-group-053']:
        for key in ['model','descriptor']:assert sha(library/ref[key])==ref[key+'_sha256']
original_index=read(index);index_hash=sha(index)
assert not any(e['id']==asset for e in original_index['assets'])
entry=copy.deepcopy(read(R/'assets/index.json')['assets'][0])
for key in ['descriptor','model']:
    entry[key]='croisement01/'+entry[key]
updated=copy.deepcopy(original_index);updated['assets'].append(entry)
assert updated['assets'][:-1]==original_index['assets']
backup=R/'publication-backup-v1';backup.mkdir(exist_ok=False)
shutil.copy2(scene,backup/scene.name);shutil.copy2(index,backup/'asset-index.json')
write(backup/'receipt.json',dict(scene_sha256=sha(scene),index_sha256=index_hash,selected_live_references=[r for r in current_scene['assetSources'] if r['id'] in ['croisement01-group-052','croisement01-group-053']],new_asset_previously_absent=True,rollback='Restore backed-up scene and index atomically; new asset directory may remain unreferenced. Existing052/053 payloads are deliberately retained.'))
assert sha(scene)==proof['live_scene_sha256'] and sha(index)==index_hash
staging=Path(tempfile.mkdtemp(prefix='.tree18-approved-',dir=target.parent))
for p in source.iterdir():
    assert p.is_file();shutil.copy2(p,staging/p.name)
os.rename(staging,target)
validate_asset_index(library/'3d-assets',updated)
assert sha(scene)==proof['live_scene_sha256'] and sha(index)==index_hash
index_tmp=index.with_name('.index-tree18-approved.json');write(index_tmp,updated);os.replace(index_tmp,index)
assert sha(scene)==proof['live_scene_sha256']
scene_tmp=scene.with_name('.croisement01-tree18-approved.json');shutil.copy2(R/scene.name,scene_tmp);os.replace(scene_tmp,scene)
write(R/'publication.json',dict(status='installed; postpublication runtime proof pending',live_scene=str(scene),scene_sha256=sha(scene),index_sha256=sha(index),model_sha256=sha(target/'model.glb'),descriptor_sha256=sha(target/'asset.json'),backup=str(backup),unchanged_other_placements=58,all_existing_palette_entries_preserved=True))
print(R/'publication.json')
