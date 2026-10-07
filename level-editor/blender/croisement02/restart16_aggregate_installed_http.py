from pathlib import Path
import json,hashlib
R=Path(__file__).resolve().parents[3];S=R/'level-editor/work/croisement02-refinement/restart2-state';h=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();read=lambda p:json.loads(p.read_text());refs={};checks={};families={};resources={};catalog_sha='135e7e443ede18656309245a0609ee537f3d14f929baf303490099d8af177825'
def bind(p):refs[str(p.relative_to(R))]=h(p);return read(p)
manifest=bind(S/'remaining-seven-package-v2/manifest.json');expected={e['id']:e for e in manifest['entries']}
for name in ['installed41-normal-http-chunk-1-v1','installed41-normal-http-remaining5-v1']:
 b=S/name;v=bind(b/'browser/verification.json');assert v['status']=='PASS_INSTALLED41_NORMAL_HTTP_CHUNK';assert v['static_map_sha256']=='616734426ba0bb56f145790cfa9013e05703ac8ab4df341dc27f4b0cc01e63ab'
 assert bind(b/'browser/process-final.json')['exitCode']==0;assert bind(b/'browser/profile-cleanup.json')['status']=='REMOVED_AFTER_CONFIRMED_CHROME_CLOSE';catalog=bind(b/'browser/installed-catalog.json');assert catalog=={'sha256':catalog_sha,'entries':41,'old34unchanged':True};assert not bind(b/'browser/live-runtime-drift.json')['changed'];assert not bind(b/'browser/live-drift.json')['changed']
 inp=bind(b/'inputs.json');assert h(b/'inputs.json')==v['inputs_sha256'];assert inp['installed_catalog']['sha256']==catalog_sha
 for p,d in inp['files'].items():assert h(R/p)==d,p
 for row in bind(b/'browser/normal-http-resources.json'):
  if row.get('virtualProductionListing'):continue
  assert h(R/'level-editor/library'/row['path'])==row['sha256'],row['path']
  if row['path'] in resources:assert resources[row['path']]==row['sha256']
  resources[row['path']]=row['sha256']
 proof=bind(b/'browser/remaining-seven-states/verification.json');assert proof['status']=='PASS_CURRENT_LIVE_COMPACT'
 for c in proof['checks']:
  assert c['pass'] is True
  if c['name'] in checks:assert c['name']=='Map-only removes all state roots'
  checks[c['name']]=c
 for entry_id in v['selected_entry_ids']:
  assert entry_id in expected and entry_id not in families;p=b/'browser/remaining-seven-states'/(entry_id+'-terminal.json');f=bind(p);assert f['status']=='PASS_CURRENT_LIVE_FAMILY' and f['contract_sha256']==expected[entry_id]['contract']['sha256'];assert f['checks']==[c for c in proof['checks'] if c['name'].startswith(entry_id+' ')];families[entry_id]={'path':str(p.relative_to(R)),'sha256':h(p)}
assert set(families)==set(expected) and len(checks)==45
for entry_id in expected:
 for label in ['exact contract','native selection','initial','applied','reset']:assert checks[entry_id+' '+label]['pass'] is True
assert h(R/'level-editor/library/mission-states/index.json')==catalog_sha
installation=bind(S/'final-current-publication-gate-v1/installation.json');assert installation['index_sha256']==catalog_sha
for n in ['final-current-publication-gate-v1/state-proof.json','current-live-seven-smoke-summary-v1.json','packaged-contact-audit-v2/root-support-review-v1.json']:bind(S/n)
out=S/'installed41-normal-http-final-v1';out.mkdir();(out/'verification.json').write_text(json.dumps({'status':'PASS_INSTALLED41_NORMAL_HTTP','catalog_sha256':catalog_sha,'entries':41,'previous_entries_preserved':34,'all_seven_contracts':families,'checks':list(checks.values()),'normal_http_resources':resources,'evidence':refs,'scope':'Postpublication actual production /library browser route; no private library/state overlay. All7 newly installed contracts and their selectors, native selection, both physical endpoints, reset, exact fence part suppression/restoration and Map-only cleanup.45unique compact assertions supplement unchanged frozen97checks21stateviews16contacts. Successful actual HTTP resource bytes and runtime/static source pins unchanged; browser sessions cleanly exited and own profiles removed.','disclosures':'Expected thumbnail AVIF probes with WebP fallback, canceled thumbnail probes, private favicon404 and existing development warnings remain recorded. No unexpected resource failures, runtime exceptions or console.error accepted. No claim of continuous physical motion or actor simulation.'},indent=2)+'\n');print(out/'verification.json',h(out/'verification.json'))
