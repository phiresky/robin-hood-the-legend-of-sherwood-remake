"""Freeze a reviewable state publication transaction without changing the library."""
from pathlib import Path
import hashlib,json
ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'level-editor/work/croisement02-refinement/restart2-state'
STAGE=BASE/'remaining-seven-package-v2'
OUT=BASE/'remaining-seven-publication-v1'
LIB=ROOT/'level-editor/library'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def read(p):return json.loads(p.read_text())
def pin(p):return {'path':str(p.relative_to(ROOT)),'sha256':sha(p)}
manifest=read(STAGE/'manifest.json');index=LIB/'mission-states/index.json';old=read(index);new=read(STAGE/'library/mission-states/index.json')
assert sha(index)==manifest['installed_index_sha256']
assert len(old['entries'])==34 and len(new['entries'])==41
assert new['entries'][:34]==old['entries'] and new['entries'][34:]==manifest['entries']
for row in manifest['files']+[manifest['private_index']]:assert sha(STAGE/'library'/row['path'])==row['sha256']
for row in manifest['reused']:assert sha(LIB/row['path'])==row['sha256']
static=ROOT/'level-editor/work/croisement02-refinement/restart2-textures/batch10-private-level3d-exact-v2'
mapfile=static/'croisement02.rhlos-map.json';mapdata=read(mapfile)
assert sha(mapfile)==manifest['static_map_sha256']
pins=static/'placement-evidence/static-placement-pins.json';assert sha(pins)==manifest['static_placement_pins_sha256']
resources=[]
for r in mapdata.get('assetSources',[])+mapdata.get('sceneAssets',[]):
 for name in ['model','descriptor']:
  path=r[name];expected=r[name+'_sha256'];p=static/'map-assets'/path
  assert sha(p)==expected
  resources.append({'path':path,'sha256':expected})
baseline={str(p.relative_to(LIB)):sha(p) for p in (LIB/'mission-states').rglob('*') if p.is_file()}
OUT.mkdir(parents=True,exist_ok=True)
backup=OUT/'installed34-index.json'
if backup.exists():assert sha(backup)==sha(index)
else:backup.write_bytes(index.read_bytes())
plan={'status':'PREPARED_NOT_EXECUTED','scope':'Mission preview entries only. Shared publisher must publish and verify matching static map first. No source, geometry, static library or gameplay mutation by this transaction.','manifest':pin(STAGE/'manifest.json'),'staged_index':pin(STAGE/'library/mission-states/index.json'),'baseline_index':pin(backup),'installed_index_sha256':sha(index),'installed_entries':34,'result_entries':41,'added_ids':[e['id'] for e in manifest['entries']],'files':manifest['files'],'reused':manifest['reused'],'baseline_files':baseline,'static_dependency':{'map':pin(mapfile),'placement_pins':pin(pins),'live_map_path':'scenes/croisement02.rhlos-map.json','role':'Frozen tested fixture only; final approved static deltas require separately pinned final candidate and binding verification.','source_resources':resources,'fence_replacement_parts':read(pins)['fence'],'ground_scope':'Exact frozen private map receiver; newer ground derivatives require separate compatible static publication validation.'},'gates':['Complete combined actual Editor state proof PASS, all seven exact contracts and endpoint selections, exact fence suppression/restoration, final map-only cleanup.','Root visual review binds successful proof and contact evidence; partial v4 output is never accepted as complete.','Shared publisher owns transaction window and has installed matching static map plus source resources; normal HTTP static delivery proof PASS.','All staged/reused/baseline nonindex files and runtime pins rechecked immediately before switch.','Normal HTTP state verification after switch confirms41 catalog entries and representative seven new controls, static suppression/reset and unchanged installed34.'],'install_sequence':['Recheck complete proof/review gate and exact static map/resources.','Lock transaction and recheck old index hash; reject concurrent catalog changes.','Create only absent hash-named state resources/contracts; existing paths must equal expected bytes.','Verify every staged/reused byte and every original nonindex byte before switching.','Write/fsync a unique temporary index; replace index atomically LAST.','Record before/after hashes and run normal HTTP state proof.'],'rollback_sequence':['If pre-switch failure: leave index34 unchanged; retain any new orphan immutable resources as diagnostics.','If post-switch validation fails: restore saved34 index atomically ONLY if current index still exactly equals this transaction41 index.','Never overwrite concurrent index changes, delete state resources, roll back static assets or reset unrelated files.'],'final_static_rule':'Final gate pins actual published map and all final source resources, validates exact fence replacement bindings, and names compatible changed-ground/material evidence relative to this frozen proof fixture. Never require final map to falsely retain old fixture hash.', 'deferred_gate':'A final gate must bind successful actual proof/root review/static publication and runtime hashes. None are fabricated by preparation.'}
(OUT/'plan.json').write_text(json.dumps(plan,indent=2)+'\n')
print(json.dumps({'status':plan['status'],'entries':7,'new_files':len(plan['files']),'unchanged_files':len(baseline),'static_resources':len(resources),'plan_sha256':sha(OUT/'plan.json')}))
