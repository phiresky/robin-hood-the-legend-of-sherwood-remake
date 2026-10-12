"""Prepare a private C1 library with scoped Tree02 catalog rows and retained payloads."""
import copy,hashlib,json,shutil,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];R=ROOT/'level-editor/work/active/croisement01/restart2';D=R/'tree02-publication-plan-v1';S=R/'tree02-integration-batch-v1';L=ROOT/'level-editor/library';A=L/'3d-assets';O=D/'private-library'
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from asset_index import write_asset_index,validate_asset_index
IDS={'croisement01-tree-02','croisement01-group-005'}
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def write(p,d):p.write_text(json.dumps(d,separators=(',',':'))+'\n')
assert not O.exists();assert shutil.disk_usage(D).free>=10*1024**3;assert next(int(x.split()[1]) for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:'))>=6*1024**2
proof=json.loads((S/'scene-splice-proof.json').read_text());assert sha(L/'scenes/croisement01.rhlos-map.json')==proof['live_scene_sha256'];index_sha=sha(A/'index.json');old=json.loads((A/'index.json').read_text());files={}
for aid in IDS:
 for name in ['model.glb','asset.json']:files[f'croisement01/{aid}/{name}']=S/'assets'/aid/name
write_asset_index(A,target=D/'affected-index.json',files=files,descriptors=[f'croisement01/{aid}/asset.json' for aid in sorted(IDS)])
affected=json.loads((D/'affected-index.json').read_text());rows={x['id']:x for x in old['assets']};assert 'croisement01-tree-02' not in rows
for row in affected['assets']:rows[row['id']]=row
new={**old,'assets':[rows[k] for k in sorted(rows)]};validate_asset_index(A,new,files=files)
for row in old['assets']:
 if row['id'] not in IDS:assert rows[row['id']]==row
for directory in ['', '3d-assets','3d-assets/croisement01','scenes']:(O/directory).mkdir(parents=True,exist_ok=True)
for src in L.iterdir():
 if src.name not in ['3d-assets','scenes']:(O/src.name).symlink_to(src.resolve(),target_is_directory=src.is_dir())
for src in A.iterdir():
 if src.name not in ['index.json','croisement01']:(O/'3d-assets'/src.name).symlink_to(src.resolve(),target_is_directory=src.is_dir())
for src in (A/'croisement01').iterdir():
 if src.name not in IDS:(O/'3d-assets/croisement01'/src.name).symlink_to(src.resolve(),target_is_directory=src.is_dir())
for aid in IDS:(O/'3d-assets/croisement01'/aid).symlink_to((S/'assets'/aid).resolve(),target_is_directory=True)
for src in (L/'scenes').iterdir():
 if src.name!='croisement01.rhlos-map.json':(O/'scenes'/src.name).symlink_to(src.resolve())
(O/'scenes/croisement01.rhlos-map.json').symlink_to((S/'croisement01.rhlos-map.json').resolve());write(O/'3d-assets/index.json',new)
assert sha(A/'index.json')==index_sha;assert sha(L/'scenes/croisement01.rhlos-map.json')==proof['live_scene_sha256']
write(D/'private-library-proof.json',dict(status='PASS_PRIVATE_LIBRARY_CATALOG',canonical_index_sha256=index_sha,private_index_sha256=sha(O/'3d-assets/index.json'),unchanged_catalog_rows=len(old['assets'])-1,changed_existing_ids=['croisement01-group-005'],added_ids=['croisement01-tree-02'],scoped_index_generation=True,full_catalog_validation=True,canonical_writes=False));print(O)
