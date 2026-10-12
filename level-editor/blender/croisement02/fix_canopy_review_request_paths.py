"""Create a new fixture with absolute /@fs resources; retain failed evidence."""
from pathlib import Path
import json,hashlib,subprocess,shutil
R=Path.cwd();B=R/'level-editor/work/croisement02-refinement';OLD=B/'canopy04-complete-review-playerfix-oct12-v1';O=B/'canopy04-complete-review-playerfix-http-v2';sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();O.mkdir(exist_ok=False)
def walk(value):
 if isinstance(value,dict):
  if isinstance(value.get('path'),str)and isinstance(value.get('sha256'),str):
   p=Path(value['path']).resolve();assert sha(p)==value['sha256'];value['path']=str(p)
  for v in value.values():walk(v)
 elif isinstance(value,list):
  for v in value:walk(v)
request=json.loads((OLD/'request.json').read_text());walk(request);(O/'request.json').write_text(json.dumps(request,indent=2)+'\n')
for name in ['run.mjs','proof.ts','build.mjs']:(O/name).write_text((OLD/name).read_text().replace(OLD.name,O.name))
subprocess.run(['node',str(O/'build.mjs')],check=True);subprocess.run(['node','--check',str(O/'run.mjs')],check=True)
pins=json.loads((O/'source-pins.json').read_text());oldpins=json.loads((OLD/'source-pins.json').read_text());pins.update(oldpins)
for name in ['request.json','proof.ts','proof.js','index.html','run.mjs','build.mjs']:pins[str(O/name)]=sha(O/name)
for row in request['rows']:
 for key in ['model','views','export','triangle_audit']:pins[row[key]['path']]=row[key]['sha256']
# Routing check uses the same path rule as the fixture server, without starting a server.
for row in request['rows']:
 for key in ['model','views']:
  url='/@fs'+row[key]['path'];assert url.startswith('/@fs/');resolved=Path(url[4:]).resolve();assert resolved.is_relative_to(R)and sha(resolved)==row[key]['sha256']
(O/'source-pins.json').write_text(json.dumps(pins,indent=2)+'\n');handoff=json.loads((OLD/'handoff.json').read_text());handoff['status']='PREPARED_HTTP_PATH_FIX_NOT_LAUNCHED';handoff['runner']={'path':str(O/'run.mjs'),'sha256':sha(O/'run.mjs')};handoff['pins']={'path':str(O/'source-pins.json'),'sha256':sha(O/'source-pins.json')};handoff['previous_request']={'path':str(OLD/'request.json'),'sha256':sha(OLD/'request.json')};handoff['candidate_identity']='All candidate/native/camera resource bytes unchanged. Pin paths normalized absolute for /@fs serving; old failure retained. CPU routing checks pass; actual HTTP pending root launch.';(O/'handoff.json').write_text(json.dumps(handoff,indent=2)+'\n');print(sha(O/'handoff.json'))
