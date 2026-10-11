"""Rebase approved climbing and Tree42 browser proposals over installed state entries."""
import json,hashlib
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/croisement02-refinement';OUT=BASE/'restart72-climbing-tree42-ui-rebase-v1';INDEX=ROOT/'level-editor/library/mission-states/index.json'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
pin=lambda p:dict(path=str(p),sha256=sha(p))
def main():
 assert not OUT.exists();OUT.mkdir();before=json.loads(INDEX.read_text());entries=list(before['entries']);routes={};steps=[]
 for name,stage,check in [('climbing','restart59-climbing-catalog-stage-v1','restart59_climbing_ui_check.js'),('tree42','restart62-tree42-catalog-stage-v2','restart62_tree42_ui_check.js')]:
  base=BASE/stage;plan=json.loads((base/'plan.json').read_text());request=json.loads((base/'browser-request.json').read_text());additions=plan.get('catalog_additions',plan.get('entries'))
  if additions is None:
   candidate=json.loads(Path(request['routes']['mission-states/index.json']['path']).read_text());oldids={x['id'] for x in entries};additions=[x for x in candidate['entries'] if x['id'].startswith('reviewed-tree42-motion-') and x['id'] not in oldids]
  assert len(additions)==(2 if name=='climbing' else 1);assert not {x['id'] for x in entries}&{x['id'] for x in additions};entries+=additions
  for destination,p in request['routes'].items():
   if destination=='mission-states/index.json':continue
   assert sha(Path(p['path']))==p['sha256'];assert destination not in routes;routes[destination]=p
  steps.append(dict(name=name,plan=pin(base/'plan.json'),check=pin(Path(__file__).with_name(check)),config=pin(base/'check-config.json'),entry_ids=[x['id'] for x in additions]))
 index=OUT/'index.json';index.write_text(json.dumps({**before,'entries':entries},indent=2)+'\n');routes['mission-states/index.json']=pin(index)
 request=dict(status='QUEUED_COMBINED_STATE_UI_PRIVATE_OVERLAY',canonical_catalog=pin(INDEX),preserved_entries=len(before['entries']),candidate_entries=len(entries),routes=routes,steps=steps,execution='One actual editor startup. For each step assign its config to window.stateUiPacket, execute its check and preserve separate result/evidence before the next. scatterStage/scatterCaptures handshake. Both checks retire to map-only. No runtime edits.',scope='Approved climbing two controls and approved Tree42 S03 loop. Exact prior103 canonical entries preserved; private overlay appends3.',canonical_writes=False)
 (OUT/'browser-request.json').write_text(json.dumps(request,indent=2)+'\n');print(pin(OUT/'browser-request.json'))
if __name__=='__main__':main()
