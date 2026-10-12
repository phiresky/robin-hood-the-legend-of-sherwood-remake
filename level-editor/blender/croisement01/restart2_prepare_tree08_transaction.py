"""Prepare the guarded native Tree08 publisher; never applies canonical writes."""
from pathlib import Path
r=Path(__file__).resolve().parents[3]/'level-editor/work/active/croisement01/restart2';d=r/'tree08-publication-plan-v1';d.mkdir();s=(r/'tree02-publication-plan-v1/transaction.py').read_text().replace("S=D.parent/'tree02-integration-batch-v1'; B=D.parent/'tree02-browser-private-v1'","S=D.parent/'tree08-native33-integration-stage-v2'; B=D.parent/'tree08-browser-private-v1'").replace("IDS={'croisement01-tree-02','croisement01-group-005'}","IDS={'croisement01-tree-08'}")
a=s.index('def evidence(');b=s.index('\ndef strict(',a)
s=s[:a]+'''def evidence(require_browser=False):
 proof=json.loads((S/'stage-proof.json').read_text())
 check(core.sha(L/'scenes/croisement01.rhlos-map.json')==proof['source_map_sha256'],'Affected live map changed; restage required')
 check(core.sha(S/'croisement01.rhlos-map.json')==proof['staged_map_sha256'],'Staged map changed')
 for field in ['model','descriptor']:
  check(core.sha(L/proof['old_source'][field])==proof['old_source'][field+'_sha256'],'Old native033 changed')
  check(core.sha(S/'assets/croisement01-tree-08'/Path(proof['new_source'][field]).name)==proof['new_source'][field+'_sha256'],'Approved payload changed')
 check(proof['new_source']['model_sha256']=='e0d5f19f4625888f3b2dfbff9e58ed0ce11a81e54b6f6d0a3aeec726846ffe8b','Wrong approved model')
 old=json.loads((L/proof['old_source']['descriptor']).read_text());new=json.loads((S/'assets/croisement01-tree-08/asset.json').read_text())
 for field in ['parts','gameplay','source_origin_scene']:check(old[field]==new[field],'Native metadata changed '+field)
 target=A/'croisement01/croisement01-tree-08';check(not target.exists() or not any(target.iterdir()),'New target already exists')
 if require_browser:
  hp=B/'handoff.json';check(hp.exists(),'Normal editor browser proof required');h=json.loads(hp.read_text());check(h['status'].startswith('PASS'),'Browser proof not passing')
  check(h['stage_scene_sha256']==proof['staged_map_sha256'],'Browser tested another map')
  check(h['model_sha256']==proof['new_source']['model_sha256'],'Browser tested another model')
  check(bool(h['evidence']),'Browser evidence missing')
  for path,digest in h['evidence'].items():check(core.sha(R/path)==digest,'Browser evidence drift: '+path)
  return h
 return proof

def preserve(current,staged):
 check({k:v for k,v in current.items() if k not in ['assetSources','placements']}=={k:v for k,v in staged.items() if k not in ['assetSources','placements']},'Unrelated map metadata changed')
 old={x['id']:x for x in current['assetSources']};new={x['id']:x for x in staged['assetSources']}
 check(set(new)-set(old)=={'croisement01-tree-08'} and set(old)-set(new)=={'croisement01-group-033'},'Wrong source membership')
 for aid,row in old.items():
  if aid!='croisement01-group-033':check(row==new[aid],'Unrelated reference drift '+aid)
 for a,b in zip(current['placements'],staged['placements'],strict=True):
  if a['id']=='group-033':check(a['assets']==['croisement01-group-033'] and b['assets']==['croisement01-tree-08'] and {k:v for k,v in a.items() if k!='assets'}=={k:v for k,v in b.items() if k!='assets'},'Native033 placement changed')
  else:check(a==b,'Unrelated placement changed')
 return {'unchanged_placements':len(current['placements'])-1,'replaced_visual':1,'native_gameplay_exact':True}
''' +s[b:]
s=s.replace("new=merge_catalog(old,affected)","new={**old,'assets':[affected['assets'][0] if row['id']=='croisement01-group-033' else row for row in old['assets']]}\n newrows={row['id']:row for row in new['assets']}")
s=s.replace("check([r['id'] for r in new['assets'] if r['id'] in oldrows]==[r['id'] for r in old['assets']],'Existing catalog entry order drift')","check([r['id'] for r in new['assets'] if r['id']!='croisement01-tree-08']==[r['id'] for r in old['assets'] if r['id']!='croisement01-group-033'],'Unrelated catalog entry order drift')")
s=s.replace("check(set(newrows)-set(oldrows)=={'croisement01-tree-02'} and not set(oldrows)-set(newrows),'Index membership drift')","check(set(newrows)-set(oldrows)=={'croisement01-tree-08'} and set(oldrows)-set(newrows)=={'croisement01-group-033'},'Index membership drift')")
s=s.replace("if aid not in IDS: check(row==newrows[aid]","if aid not in IDS and aid!='croisement01-group-033': check(row==newrows[aid]")
s=s.replace("strict(folder,D/'private-library')","strict(folder,B/'private-library')").replace("pin(S/'scene-splice-proof.json')","pin(S/'stage-proof.json')").replace('descriptors=two IDs','descriptors=one ID; replaces only native033 catalog row')
(d/'transaction.py').write_text(s);(d/'validate.mjs').write_text((r/'tree02-publication-plan-v1/validate.mjs').read_text());print(d)
