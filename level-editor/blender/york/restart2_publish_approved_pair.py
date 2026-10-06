"""Publish only the reviewed York pair after exact current-baseline guards."""
import copy,hashlib,json,os,shutil
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/york-refinement/restart2/pair-textures-v1/assembled-v3-bounded/export-v2'
sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
plan_path=BASE/'publication-plan-v1.json';assert sha(plan_path)=='fe8aa348bdc09669aff7b2dc47fac6447f048d495fd6bfbd12a136eee8dea7cd';plan=json.loads(plan_path.read_text())
proof=json.loads(Path(plan['proof']).read_text());assert sha(plan['proof'])==plan['proof_sha256']
for path,digest in proof['evidence'].items():assert sha(path)==digest
index_path=Path(plan['index']['path']);scene_path=Path(plan['scene']['path']);index=json.loads(index_path.read_text());scene=json.loads(scene_path.read_text())
baseline=json.loads((BASE/'editor-stage-v8/full-map/york.rhlos-map.json').read_text())
for patch in plan['scene']['asset_source_patches']:
 i=next(i for i,r in enumerate(baseline['assetSources'])if r['id']==patch['asset_id']);baseline['assetSources'][i]=patch['expected_before']
allowed={'york-cathedral-precinct-raised-terrain':'a62d9360be38f5a02dab74858d476daaff377466c21bb98e7bae59269120ca8c','york-precinct-southwest-wall-ramp':'79980aa543366e3e91858f89dbb0cf5dad9146dd6e2184917755329d8d9f0ff4'}
for ref in baseline['assetSources']:
 if ref['id']in allowed:ref['descriptor_sha256']=allowed[ref['id']]
assert baseline==scene,'Unreviewed scene drift'
new_index=copy.deepcopy(index);new_scene=copy.deepcopy(scene)
for patch in plan['index']['entry_patches']:
 i=next(i for i,r in enumerate(index['assets'])if r['id']==patch['asset_id']);assert index['assets'][i]==patch['expected_before'];new_index['assets'][i]=patch['replacement']
for patch in plan['scene']['asset_source_patches']:
 i=next(i for i,r in enumerate(scene['assetSources'])if r['id']==patch['asset_id']);assert scene['assetSources'][i]==patch['expected_before'];new_scene['assetSources'][i]=patch['replacement']
for row in plan['file_operations']:
 assert sha(row['source'])==row['source_sha256'];assert sha(row['destination'])==row['expected_before_sha256']
 if Path(row['destination']).name=='asset.json':assert json.loads(Path(row['source']).read_text())['gameplay']==json.loads(Path(row['destination']).read_text())['gameplay']
output=BASE/'publication-v1';output.mkdir();backup=output/'rollback-current-baseline';backup.mkdir()
shutil.copyfile(index_path,backup/'index.json');shutil.copyfile(scene_path,backup/'york.rhlos-map.json')
for row in plan['file_operations']:
 dst=Path(row['destination']);saved=backup/dst.parent.name/dst.name;saved.parent.mkdir(exist_ok=True);shutil.copyfile(dst,saved)
new_scene_path=output/'york.rhlos-map.json';new_scene_path.write_text(json.dumps(new_scene,indent=2)+'\n')
new_index_path=output/'index.json';new_index_path.write_text(json.dumps(new_index,separators=(',',':'))+'\n')
assert sha(index_path)==sha(backup/'index.json') and sha(scene_path)==sha(backup/'york.rhlos-map.json')
for row in plan['file_operations']:
 dst=Path(row['destination']);tmp=dst.with_name(dst.name+'.york-approved-pair.tmp');assert not tmp.exists();shutil.copyfile(row['source'],tmp);os.replace(tmp,dst)
for src,dst in [(new_scene_path,scene_path),(new_index_path,index_path)]:
 tmp=dst.with_name(dst.name+'.york-approved-pair.tmp');assert not tmp.exists();shutil.copyfile(src,tmp);os.replace(tmp,dst)
for row in plan['file_operations']:assert sha(row['destination'])==row['source_sha256']
assert json.loads(index_path.read_text())==new_index and json.loads(scene_path.read_text())==new_scene
(output/'publication.json').write_text(json.dumps({'status':'Installed exact approved pair; normal HTTP proof pending','plan_sha256':sha(plan_path),'proof_sha256':sha(plan['proof']),'root_authorized_baseline_refresh':allowed,'rollback':str(backup),'published_files':{r['destination']:r['source_sha256']for r in plan['file_operations']},'index_sha256':sha(index_path),'scene_sha256':sha(scene_path),'placements_unchanged':True,'unrelated_index_entries_preserved':True,'gameplay_preserved':True,'model_choice':'Exact full approved pair; pair lossy references removed; no file deletion'},indent=2)+'\n')
print(output)
