import json,hashlib
from pathlib import Path
root=Path('level-editor/work/croisement02-refinement')
src=root/'restart2-state/state-preview-ui-v2'
out=root/'restart2-state/mission-state-package-v1'
lib=out/'library'
sha=lambda b:hashlib.sha256(b).hexdigest()
records={}
def write(path,data,source=None):
 p=lib/path;p.parent.mkdir(parents=True,exist_ok=True)
 if p.exists():assert p.read_bytes()==data,path
 else:p.write_bytes(data)
 records[path]={'path':path,'sha256':sha(data),'bytes':len(data),'source':str(source)if source else None}
 return {'path':path,'sha256':sha(data)}
def resource(path,expected):
 p=root/path;b=p.read_bytes();assert sha(b)==expected,p
 target='mission-states/croisement02/resources/'+expected+p.suffix.lower()
 return write(target,b,p)['path']
def walk(x):
 if isinstance(x,list):
  for v in x:walk(v)
 elif isinstance(x,dict):
  if isinstance(x.get('path'),str)and'sha256'in x:x['path']=resource(x['path'],x['sha256'])
  if isinstance(x.get('model'),str)and'model_sha256'in x:x['model']=resource(x['model'],x['model_sha256'])
  for v in x.values():walk(v)
index=json.loads((src/'index.json').read_text())
for e in index['entries']:
 for k in ['mission_data','level_data']:
  p=root/e[k]['path'];b=p.read_bytes();assert sha(b)==e[k]['sha256'];e[k]=write('mission-states/croisement02/source/'+p.name,b,p)
 p=root/e['contract']['path'];b=p.read_bytes();assert sha(b)==e['contract']['sha256'];c=json.loads(b);walk(c)
 e['contract']=write('mission-states/croisement02/contracts/'+e['id']+'.json',(json.dumps(c,indent=2)+'\n').encode(),p)
write('mission-states/index.json',(json.dumps(index,indent=2)+'\n').encode())
report={'scope':'Private immutable mission state package; no canonical installation','entries':[e['id']for e in index['entries']],'resources':list(records.values()),'total_bytes':sum(r['bytes']for r in records.values()),'source_ui_proof':str(src/'self-review.json'),'source_ui_proof_sha256':sha((src/'self-review.json').read_bytes()),'guards':'Every source byte hash checked; JSON paths alone relocated. Common family origins and explicit world translations unchanged.'}
(out/'manifest.json').write_text(json.dumps(report,indent=2)+'\n')
print(len(records),'files',report['total_bytes'],'bytes')
