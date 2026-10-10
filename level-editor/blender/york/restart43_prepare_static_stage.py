"""Prepare compact scoped York asset/map/catalog deltas; never mutate canonical files."""
import copy,hashlib,json,shutil,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/york-refinement/restart2';LIB=ROOT/'level-editor/library';ASSETS=LIB/'3d-assets';OUT=BASE/'restart43-approved-static-stage-v3';assert not OUT.exists()
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from asset_index import write_asset_index
CONFIG={'shed':'restart43-approved-shed-export-v1','well':'restart42-approved-well-export-v2','storehouse':'restart42-approved-storehouse-export-v1','gatehouse':'restart43-approved-gatehouse-export-v1'}
sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
def pin(p):return dict(path=str(p),sha256=sha(p),bytes=p.stat().st_size)
def dump(p,d):p.parent.mkdir(parents=True,exist_ok=True);p.write_text(json.dumps(d,indent=2)+'\n')
assert shutil.disk_usage(ROOT).free>10*1024**3
catalog_path=ROOT/'level-editor/refinement/catalogs/york.json';old_catalog=json.loads(catalog_path.read_text());catalog=copy.deepcopy(old_catalog)
map_path=LIB/'scenes/york.rhlos-map.json';old_map=json.loads(map_path.read_text());scene=copy.deepcopy(old_map)
records=[];protected=[pin(map_path),pin(catalog_path)];files={};ids=set();payloads={}
OUT.mkdir()
for key,directory in CONFIG.items():
 source=BASE/directory;handoff=json.loads((source/'integration-handoff.json').read_text());aid=handoff['asset_id'];ids.add(aid)
 for file,digest in handoff['files'].items():assert sha(file)==digest,(file,'Frozen file changed')
 protected.extend([pin(source/'integration-handoff.json'),pin(source/'browser-request.json')]);old_descriptor=ASSETS/'york'/aid/'asset.json';assert sha(old_descriptor)==handoff.get('gameplay_matches_live_descriptor_sha256',handoff.get('live_pins',{}).get(str(old_descriptor)))
 incoming=OUT/'assets'/aid;incoming.mkdir(parents=True);payloads[aid]=incoming
 for name in ['model.glb','asset.json']:
  original=source/'3d-assets'/aid/name;target=ASSETS/'york'/aid/name;dest=incoming/name;shutil.copyfile(original,dest)
  records.append(dict(source=str(dest),target=str(target),old_sha256=sha(target),new_sha256=sha(dest),operation='replace'))
  files[f'york/{aid}/{name}']=dest
 for name in ['lossy.glb','lossy.glb.receipt.json','preview.glb','preview.glb.receipt.json']:
  target=ASSETS/'york'/aid/name
  if target.exists():records.append(dict(source=None,target=str(target),old_sha256=sha(target),new_sha256=None,operation='remove_obsolete_derivative'))
  files[f'york/{aid}/{name}']=None
 new_descriptor=json.loads((incoming/'asset.json').read_text());old=json.loads(old_descriptor.read_text());assert new_descriptor['gameplay']==old['gameplay'];assert new_descriptor['source_origin_scene']==old['source_origin_scene']
 ref=next(r for r in scene['assetSources']if r['id']==aid);assert ref['model_sha256']==sha(ASSETS/ref['model'].removeprefix('3d-assets/'));assert ref['descriptor_sha256']==sha(old_descriptor)
 ref['model_sha256']=sha(incoming/'model.glb');ref['descriptor_sha256']=sha(incoming/'asset.json')
 private_catalog=json.loads((source/'private-catalog.json').read_text());group=next(g for g in private_catalog['groups']if g['id']==aid)
 old_group=next(g for g in old_catalog['groups']if g['id']==aid);assert {k:v for k,v in group.items()if k!='parts'}=={k:v for k,v in old_group.items()if k!='parts'}
 # Source nodes and all prior semantic part metadata are retained; only approved component identifiers expand.
 assert len(group['parts'])==len(old_group['parts'])
 for before,after in zip(old_group['parts'],group['parts']):assert {k:v for k,v in before.items()if k!='components'}=={k:v for k,v in after.items()if k!='components'}
 catalog['groups'][next(i for i,g in enumerate(catalog['groups'])if g['id']==aid)]=group
assert scene['placements']==old_map['placements']
assert {k:v for k,v in scene.items()if k!='assetSources'}=={k:v for k,v in old_map.items()if k!='assetSources'}
for before,after in zip(old_map['assetSources'],scene['assetSources']):
 if before['id']not in ids:assert before==after
 else:assert {k:v for k,v in before.items()if k not in ['model_sha256','descriptor_sha256']}=={k:v for k,v in after.items()if k not in ['model_sha256','descriptor_sha256']}
for before,after in zip(old_catalog['groups'],catalog['groups']):
 if before['id']not in ids:assert before==after
assert {k:v for k,v in old_catalog.items()if k!='groups'}=={k:v for k,v in catalog.items()if k!='groups'}
staged_map=OUT/'york.rhlos-map.json';dump(staged_map,scene);staged_catalog=OUT/'york-catalog.json';dump(staged_catalog,catalog)
for src,target in [(staged_map,map_path),(staged_catalog,catalog_path)]:records.append(dict(source=str(src),target=str(target),old_sha256=sha(target),new_sha256=sha(src),operation='replace'))
write_asset_index(ASSETS,target=OUT/'affected-index.json',files=files,descriptors=[f'york/{aid}/asset.json'for aid in sorted(ids)])
rows=json.loads((OUT/'affected-index.json').read_text())['assets'];assert {r['id']for r in rows}==ids;assert all(not any(k in r for k in ['lossy_model','preview_model'])for r in rows)
# A private library resolves unchanged sources by symlink and only owns the replacement folders.
private=OUT/'library';private.mkdir()
for entry in LIB.iterdir():
 if entry.name not in ['3d-assets','scenes']:(private/entry.name).symlink_to(entry,target_is_directory=entry.is_dir())
(private/'scenes').mkdir();(private/'scenes/york.rhlos-map.json').symlink_to(staged_map)
(private/'3d-assets').mkdir()
for entry in ASSETS.iterdir():
 if entry.name=='index.json':continue
 if entry.name!='york':(private/'3d-assets'/entry.name).symlink_to(entry,target_is_directory=entry.is_dir())
york=private/'3d-assets/york';york.mkdir()
for entry in (ASSETS/'york').iterdir():(york/entry.name).symlink_to(payloads.get(entry.name,entry),target_is_directory=entry.is_dir())
plan=dict(status='SCOPED_STAGE_READY_FOR_LOADER_AND_ROOT_TRANSACTION_REVIEW',canonical_writes=False,asset_ids=sorted(ids),records=records,protected=protected,affected_index=pin(OUT/'affected-index.json'),index_policy='At root apply, merge these source-of-truth affected rows into the CURRENT live index under publication lock. Preserve every unrelated row and top-level field exactly; validate complete merged index, then install index last. Do not reuse an earlier full catalog snapshot.',derivative_policy='Supported original-model path; old affected lossy/preview payloads and receipts explicitly removed. No fabricated receipts.',preserved_placements=len(scene['placements']),preserved_unrelated_map_references=len(scene['assetSources'])-len(ids),preserved_unrelated_catalog_groups=len(catalog['groups'])-len(ids),remaining=['Strict production stored-map loader validation','Queued exact-model browser inspection','Root transaction implementation/dry-run/apply with rollback and preserved unrelated catalog rows','Fresh live map/group/component/palette verification after publication'])
dump(OUT/'plan.json',plan);print(OUT,sha(OUT/'plan.json'))
