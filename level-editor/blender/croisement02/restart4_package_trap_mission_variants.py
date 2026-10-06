"""Stage verified mission variants and retain every installed resource unchanged."""
from pathlib import Path
import hashlib,json
root=Path('level-editor');work=root/'work/croisement02-refinement';source=work/'restart2-state/trap-mission-variants-v1';out=work/'restart2-state/trap-mission-package-v1';library=root/'library';out.mkdir(exist_ok=True)
sha=lambda data:hashlib.sha256(data).hexdigest()
manifest=json.loads((source/'manifest.json').read_text());index_path=library/'mission-states/index.json';index=json.loads(index_path.read_text());installed_entries=json.loads(index_path.read_text())['entries'];files={};reused={};rows=[]
def put(path,data):
 p=out/'library'/path;p.parent.mkdir(parents=True,exist_ok=True)
 if p.exists():assert p.read_bytes()==data
 else:p.write_bytes(data)
 files[path]={'path':path,'sha256':sha(data),'bytes':len(data)};return{'path':path,'sha256':sha(data)}
def refs(value):
 if isinstance(value,list):
  for item in value:refs(item)
 elif isinstance(value,dict):
  path=value.get('model')or value.get('path');digest=value.get('model_sha256')or value.get('sha256')
  if isinstance(path,str)and isinstance(digest,str):
   data=(library/path).read_bytes();assert sha(data)==digest
   reused[path]={'path':path,'sha256':digest,'bytes':len(data)}
  for item in value.values():refs(item)
for row in manifest['records']:
 if row.get('status')=='HOLD':continue
 data=(source/row['contract']).read_bytes();assert sha(data)==row['sha256'];contract=json.loads(data);refs(contract)
 original=next(e for e in installed_entries if e['id']==row['family']);mission_file=library/'game-data/Data/Levels'/(row['mission']+'.rhm.json');mission_pin=put('mission-states/croisement02/source/'+mission_file.name,mission_file.read_bytes())
 entry={'kind':'transition','id':row['mission'].lower()+'-'+row['family'],'name':original['name'],'map':'Croisement02','mission':row['mission'],'contract':put('mission-states/croisement02/contracts/'+row['contract'],data),'mission_data':mission_pin,'level_data':original['level_data']}
 assert entry['id']not in[e['id']for e in index['entries']];index['entries'].append(entry);rows.append(dict(row,entry=entry))
assert len(rows)==11
put('mission-states/index.json',(json.dumps(index,indent=2)+'\n').encode())
result={'status':'PRIVATE PREPARED; NO INSTALLATION OR BROWSER LAUNCH','previous_index_sha256':sha(index_path.read_bytes()),'source_manifest_sha256':sha((source/'manifest.json').read_bytes()),'entries':rows,'new_files':list(files.values()),'reused_files':list(reused.values()),'new_bytes':sum(r['bytes']for r in files.values()),'installed_entry_preservation':index['entries'][:len(installed_entries)]==installed_entries,'holds':['S03 log tie remains excluded; no validator relaxation.','Per-mission source validation only; actual Editor review and physical endpoint context integration remain.','Duplicate rock profile targets are listed in source audit, not jointly activated by assumption.'],'scope':'Source JSON and contracts only; all artwork and approved GLB bytes reused from installed library.'}
(out/'manifest.json').write_text(json.dumps(result,indent=2)+'\n');print(len(files),'new private JSON files',result['new_bytes'],'bytes;',len(reused),'installed resources verified')
