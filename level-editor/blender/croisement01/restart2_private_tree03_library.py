"""Build a read-only private library overlay for the exact approved batch check."""
import copy,json,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from review_evidence import sha
stage=ROOT/'level-editor/work/croisement01-refinement/restart2/tree03-integration-batch-v1';live=ROOT/'level-editor/library';out=stage/'private-library-v1';out.mkdir(exist_ok=False)
for entry in live.iterdir():
 if entry.name not in ['3d-assets','scenes']:(out/entry.name).symlink_to(entry,target_is_directory=entry.is_dir())
(out/'scenes').mkdir()
for entry in (live/'scenes').iterdir():
 if entry.name!='croisement01.rhlos-map.json':(out/'scenes'/entry.name).symlink_to(entry,target_is_directory=entry.is_dir())
(out/'scenes/croisement01.rhlos-map.json').symlink_to(stage/'croisement01.rhlos-map.json')
(out/'3d-assets').mkdir();(out/'3d-assets/croisement01').mkdir()
for entry in (live/'3d-assets').iterdir():
 if entry.name not in ['croisement01','index.json','blobs']:(out/'3d-assets'/entry.name).symlink_to(entry,target_is_directory=entry.is_dir())
(out/'3d-assets/blobs').mkdir()
for entry in (live/'3d-assets/blobs').iterdir():(out/'3d-assets/blobs'/entry.name).symlink_to(entry,target_is_directory=entry.is_dir())
for entry in (ROOT/'level-editor/work/croisement01-refinement/restart2/tree03-visual-transfer-v1/3d-assets/blobs').iterdir():
 target=out/'3d-assets/blobs'/entry.name
 if not target.exists():target.symlink_to(entry)
staged=json.loads((stage/'assets/index.json').read_text())['assets'];staged_ids={e['id'] for e in staged}
for entry in (live/'3d-assets/croisement01').iterdir():
 if entry.name not in staged_ids:(out/'3d-assets/croisement01'/entry.name).symlink_to(entry,target_is_directory=entry.is_dir())
index=json.loads((live/'3d-assets/index.json').read_text())
index['assets']=[e for e in index['assets'] if e['id'] not in staged_ids]
for entry in staged:
 (out/'3d-assets/croisement01'/entry['id']).symlink_to(stage/'assets'/entry['id'],target_is_directory=True)
 e=copy.deepcopy(entry)
 for key in ['descriptor','model']:e[key]='croisement01/'+e[key]
 index['assets'].append(e)
(out/'3d-assets/index.json').write_text(json.dumps(index,indent=2)+'\n')
(stage/'private-library-proof.json').write_text(json.dumps(dict(status='private overlay only; no canonical writes',live_scene_sha256=sha(live/'scenes/croisement01.rhlos-map.json'),staged_scene_sha256=sha(stage/'croisement01.rhlos-map.json'),overlay_index_sha256=sha(out/'3d-assets/index.json')),indent=2)+'\n')
print(out)
