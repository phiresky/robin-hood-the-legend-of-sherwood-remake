"""Prepare, then publish the reviewed hall under the coordinator's writer slot."""
import argparse, copy, gzip, hashlib, importlib.util, json, os, shutil
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
BASE = ROOT / 'level-editor/work/york-refinement/restart2/hall-textures-v2/exports-v3/independent-wiring-v2'
LIB = ROOT / 'level-editor/library'
ASSETS = LIB / '3d-assets'
ID = 'york-castle-great-hall'
sha = lambda p: hashlib.sha256(Path(p).read_bytes()).hexdigest()
parser = argparse.ArgumentParser()
parser.add_argument('--publish', action='store_true', help='Requires the coordinator-assigned serialized publication slot')
args = parser.parse_args()
plan_path = BASE / 'publication-plan-v2.json'
assert sha(plan_path) == '8d3b167b7383cef13f849a456edfa29d5c88dcf34a11051d1f465ef939394249'
plan = json.loads(plan_path.read_text())
proof_path = Path(plan['functional_proof']['path'])
assert sha(proof_path) == plan['functional_proof']['sha256']
proof = json.loads(proof_path.read_text())
for path, digest in proof['evidence'].items():
    assert sha(path) == digest, path
assert proof['functional']['awaited_postreload']['instances'] == 3
assert proof['functional']['awaited_postreload']['independentControls'] == 6
assert shutil.disk_usage(ROOT).free >= 25 * 1024**3

index_path, scene_path = ASSETS / 'index.json', LIB / 'scenes/york.rhlos-map.json'
catalog_path = ROOT / 'level-editor/refinement/catalogs/york.json'
index, scene, catalog = [json.loads(p.read_text()) for p in (index_path, scene_path, catalog_path)]
assert sha(scene_path) == plan['current_map']['sha256'], 'Unreviewed York map drift'
assert next(e for e in index['assets'] if e['id'] == ID) == plan['current_index']['entry']
delta_path = Path(plan['catalog_delta']['path'])
assert sha(delta_path) == plan['catalog_delta']['sha256']
delta = json.loads(delta_path.read_text())
assert next(g for g in catalog['groups'] if g['id'] == ID) == delta['group_before']
files = {}
for row in plan['archive_before_install']:
    assert sha(row['source']) == row['sha256']
    files[Path(row['source']).relative_to(ASSETS).as_posix()] = None
for row in plan['payload']:
    assert sha(row['source']) == row['sha256']
    target = Path(row['target'])
    assert (sha(target) if target.exists() else None) == row['before_sha256']
    files[target.relative_to(ASSETS).as_posix()] = Path(row['source'])
new_descriptor = json.loads(files[f'york/{ID}/asset.json'].read_text())
assert new_descriptor['gameplay'] == json.loads((ASSETS / f'york/{ID}/asset.json').read_text())['gameplay']

spec = importlib.util.spec_from_file_location('hall_asset_index', ROOT / 'level-editor/refinement/asset_index.py')
module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
# Fresh full discovery is a mandatory guard, while the scoped merge preserves every unrelated entry.
fresh = module.generate_asset_index(ASSETS, files=files)
replacement = next(e for e in fresh['assets'] if e['id'] == ID)
assert 'lossy_model' not in replacement
new_index = copy.deepcopy(index)
new_index['assets'][next(i for i,e in enumerate(index['assets']) if e['id'] == ID)] = replacement
module.validate_asset_index(ASSETS, new_index, files=files)
new_scene = copy.deepcopy(scene)
reference = next(r for r in new_scene['assetSources'] if r['id'] == ID)
reference['model_sha256'] = sha(files[f'york/{ID}/model.glb'])
reference['descriptor_sha256'] = sha(files[f'york/{ID}/asset.json'])
reference.pop('model_scene', None)
placement = next(p for p in new_scene['placements'] if p['id'] == ID)
assert placement == plan['current_map']['placement']
placement.setdefault('patches', {})[ID] = {'appearance-1':'patch-001','appearance-2':'patch-002'}
new_catalog = copy.deepcopy(catalog)
new_catalog['groups'][next(i for i,g in enumerate(catalog['groups']) if g['id'] == ID)] = delta['group_after']
for node, owner in delta['canonical_owner_additions'].items():
    assert new_catalog['canonical_owners'].get(node) in (None, owner)
    new_catalog['canonical_owners'][node] = owner
before = {str(p):sha(p) for p in (index_path,scene_path,catalog_path)}
prepared = BASE / 'publication-prepared-v1'
prepared.mkdir(exist_ok=True)
index_bytes = module.encoded(new_index)
(prepared/'index.json.gz').write_bytes(gzip.compress(index_bytes,mtime=0))
for name, value in [('york.rhlos-map.json',new_scene),('catalog.json',new_catalog)]:
    (prepared/name).write_text(json.dumps(value,indent=2)+'\n')
guards = {'status':'PASS prospective full discovery and scoped merged index', 'baseline':before,
          'plan_sha256':sha(plan_path),'proof_sha256':sha(proof_path),
          'fresh_entries':len(fresh['assets']),'unrelated_index_entries_preserved':True,
          'gameplay_preserved':True,'placement_transform_preserved':True,'live_writes':False}
(prepared/'preflight.json').write_text(json.dumps(guards,indent=2)+'\n')
if not args.publish:
    print(json.dumps({'prepared':str(prepared),'live_writes':False})); raise SystemExit

out = BASE / 'publication-v1'; out.mkdir()
backup = out / 'rollback-current-baseline'; backup.mkdir()
(backup/'index.json.gz').write_bytes(gzip.compress(index_path.read_bytes(),mtime=0))
shutil.copyfile(scene_path,backup/'york.rhlos-map.json')
shutil.copyfile(catalog_path,backup/'catalog.json')
for row in plan['payload']:
    path=Path(row['target'])
    if path.exists(): shutil.copyfile(path,backup/path.name)
assert before == {str(p):sha(p) for p in (index_path,scene_path,catalog_path)}
archive = out/'obsolete-derivatives'; archive.mkdir()
for row in plan['archive_before_install']:
    source=Path(row['source']); assert sha(source)==row['sha256']; os.replace(source,archive/source.name)
def install_bytes(data, target):
    tmp=target.with_name(target.name+'.york-hall.tmp'); assert not tmp.exists()
    tmp.write_bytes(data); os.replace(tmp,target)
for row in plan['payload']: install_bytes(Path(row['source']).read_bytes(),Path(row['target']))
install_bytes((prepared/'catalog.json').read_bytes(),catalog_path)
install_bytes((prepared/'york.rhlos-map.json').read_bytes(),scene_path)
install_bytes(index_bytes,index_path)
for row in plan['payload']: assert sha(row['target'])==row['sha256']
module.validate_asset_index(ASSETS,json.loads(index_path.read_text()))
receipt={**guards,'status':'Installed scoped approved hall; installed HTTP proof pending','live_writes':True,
         'rollback':str(backup),'archive':str(archive),'payload':plan['payload'],
         'published':{str(p):sha(p) for p in (index_path,scene_path,catalog_path)},
         'scope_exclusions':plan['excluded']}
(out/'publication.json').write_text(json.dumps(receipt,indent=2)+'\n')
print(out)
