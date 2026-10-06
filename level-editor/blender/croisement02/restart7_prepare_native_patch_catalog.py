"""Prepare immutable source-preview routes without publishing library files."""
from pathlib import Path
import json,hashlib
B=Path('level-editor/work/croisement02-refinement');S=B/'restart2-state/remaining-seven-package-v2/library';P=B/'restart7-source-patch-delivery/contracts-v1';O=B/'restart7-source-patch-delivery/catalog-private-v1';O.mkdir(exist_ok=True)
def read(p):return json.loads(p.read_text())
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
manifest=read(P/'manifest.json');index=read(S/'mission-states/index.json');entries=list(index['entries']);routes={}
for r in manifest['records']:
 assert r['status']=='SOURCE_BINDINGS_PASS';source=P/r['contract'];assert sha(source)==r['sha256'];template=next(e for e in index['entries'] if e['mission']==r['mission']);path='mission-states/croisement02/contracts/source-'+r['id']+'.json';routes[path]={'source':str(source.resolve()),'sha256':sha(source)};entries.append({'id':'source-'+r['id'],'name':r['profile']+' — '+str(r['index']),'kind':'native-patch','map':'Croisement02','mission':r['mission'],'contract':{'path':path,'sha256':sha(source)},'mission_data':template['mission_data'],'level_data':template['level_data']})
for r in manifest['resources']:
 source=Path(r['source']);assert sha(source)==r['sha256'];routes[r['path']]={'source':str(source.resolve()),'sha256':r['sha256']}
for f in S.rglob('*'):
 if f.is_file():
  relative=f.relative_to(S).as_posix()
  if relative=='mission-states/index.json':continue
  pin={'source':str(f.resolve()),'sha256':sha(f)}
  if relative in routes:assert routes[relative]['sha256']==pin['sha256']
  else:routes[relative]=pin
(O/'index.json').write_text(json.dumps({'version':1,'entries':entries},indent=2)+'\n');routes['mission-states/index.json']={'source':str((O/'index.json').resolve()),'sha256':sha(O/'index.json')}
(O/'manifest.json').write_text(json.dumps({'status':'PRIVATE_ROUTES_NO_PUBLICATION','entries':len(entries),'new_entries':82,'source_patch_controls':129,'physical_completion_claim':False,'base_index_sha256':sha(S/'mission-states/index.json'),'files':routes,'fallback':'Read other paths from installed library; every declared route is hash-pinned.'},indent=2)+'\n');print(O,len(entries),len(routes))
