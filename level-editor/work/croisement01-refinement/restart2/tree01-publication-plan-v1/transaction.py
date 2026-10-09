"""Private preparation by default. Root-only apply uses the existing guarded publisher core."""
from pathlib import Path
import argparse, hashlib, importlib.util, json, shutil, subprocess, sys
R=Path(__file__).resolve().parents[5]; D=Path(__file__).resolve().parent; L=R/'level-editor/library'; A=L/'3d-assets'; S=D.parent/'tree01-integration-batch-v1'; B=D.parent/'tree01-browser-private-v1'
sys.path.insert(0,str(R/'level-editor/refinement'))
check_generator=R/'level-editor/refinement/asset_index.py'
assert hashlib.sha256(check_generator.read_bytes()).hexdigest()=='bf2860d6dcb9853c94850dac62a3181bbb692479183bbc570a9898cc51be5262'
from asset_index import write_asset_index, validate_asset_index
CORE=R/'level-editor/work/croisement02-refinement/restart25-approved-appearance-delta-v1/derivatives-v3/transaction.py'
assert hashlib.sha256(CORE.read_bytes()).hexdigest()=='1b79d5020ec58855514800d8d48b6196eefefedb19ea2ac0937245d8e609787f'
spec=importlib.util.spec_from_file_location('publication_core',CORE); core=importlib.util.module_from_spec(spec); spec.loader.exec_module(core)
IDS={'croisement01-tree-01','croisement01-group-005'}
def pin(p): return {'path':str(p.relative_to(R)), 'sha256':core.sha(p),'bytes':p.stat().st_size}
def check(ok,msg): core.require(ok,msg)
def dump(p,data): core.durable_json(p,data)
def evidence():
 check(core.sha(B/'handoff.json')=='3180eed00d23d5f7778060b09202fd507aca62dbacd385938d2e6d687667e496','Frozen handoff drift')
 h=json.loads((B/'handoff.json').read_text())
 for p,digest in h['evidence'].items(): check(core.sha(R/p)==digest,'Frozen evidence drift: '+p)
 for name,digest in {'model.glb':'a915bfd38b560c34e9320d628fc9463bd93e0edc8a53d4dfbd1ca39b59ac09ec','asset.json':'24c0066f7b054fa4038d089167bd00e11b3a6522519e83ea0b7cfea651c11dd9'}.items():
  check(core.sha(A/'croisement01/croisement01-group-005'/name)==digest,'Live retained group changed since browser proof')
 new_target=A/'croisement01/croisement01-tree-01'
 check(not new_target.exists() or not any(new_target.iterdir()),'New Tree01 target unexpectedly contains files')
 proof=json.loads((S/'scene-splice-proof.json').read_text())
 check(core.sha(L/'scenes/croisement01.rhlos-map.json')==proof['live_scene_sha256'],'Affected live map changed; new scoped splice needed')
 check(core.sha(S/'croisement01.rhlos-map.json')==proof['staged_scene_sha256'],'Staged map changed')
 for aid,v in proof['assets'].items():
  for name,key in [('model.glb','model_sha256'),('asset.json','descriptor_sha256')]: check(core.sha(S/'assets'/aid/name)==v[key],'Approved payload drift')
 # Native appearance bytes were tested; discovering a derivative would change that path.
 for aid in IDS:
  for folder in [A/'croisement01'/aid,S/'assets'/aid]:
   check(not folder.exists() or set(p.name for p in folder.iterdir())<= {'model.glb','asset.json'},'Unexpected affected asset payload/derivative: '+str(folder))
 return h

def preserve(current, staged):
 check({k:v for k,v in current.items() if k not in ['assetSources','placements']}=={k:v for k,v in staged.items() if k not in ['assetSources','placements']},'Map unrelated metadata changed')
 old={x['id']:x for x in current['assetSources']}; new={x['id']:x for x in staged['assetSources']}
 check(set(new)-set(old)=={'croisement01-tree-01'} and not set(old)-set(new),'Wrong source membership')
 for aid,row in old.items():
  if aid!='croisement01-group-005': check(row==new[aid],'Unrelated map reference drift '+aid)
 oldp={x['id']:x for x in current['placements']};newp={x['id']:x for x in staged['placements']}
 check(set(newp)-set(oldp)=={'tree01-wood-approved'} and all(newp[k]==v for k,v in oldp.items()),'Placement changed')
 return {'unchanged_placements':len(oldp),'added_placements':1,'unchanged_references':len(old)-1}

def strict(folder, library):
 result=subprocess.run(['node',str(D/'validate.mjs'),str(S/'croisement01.rhlos-map.json'),str(library)],cwd=R,capture_output=True,text=True)
 check(result.returncode==0,'Strict loader failed: '+result.stderr)
 dump(folder/'strict-loader.json',json.loads(result.stdout))

def prepare(folder):
 h=evidence(); folder.mkdir(exist_ok=False)
 preservation=preserve(json.loads((L/'scenes/croisement01.rhlos-map.json').read_text()),json.loads((S/'croisement01.rhlos-map.json').read_text()))
 # Preserve all current metadata; generated source-of-truth rows must be exact outside scope.
 old=json.loads((A/'index.json').read_text()); index_pin=pin(A/'index.json'); files={}; records=[]
 def add(src,target): records.append({'source':str(src),'target':str(target),'old_sha256':core.sha(target),'new_sha256':core.sha(src)})
 for aid in sorted(IDS):
  for name in ['model.glb','asset.json']:
   src=S/'assets'/aid/name;target=A/'croisement01'/aid/name;files[str(target.relative_to(A))]=src; add(src,target)
 write_asset_index(A,target=folder/'affected-index.json',files=files,descriptors=[f'croisement01/{aid}/asset.json' for aid in sorted(IDS)])
 affected=json.loads((folder/'affected-index.json').read_text())
 oldrows={x['id']:x for x in old['assets']};newrows=dict(oldrows)
 for row in affected['assets']:newrows[row['id']]=row
 check({row['id'] for row in affected['assets']}==IDS,'Wrong scoped generation')
 new={**old,'assets':[newrows[k] for k in sorted(newrows)]}
 validate_asset_index(A,new,files=files)
 dump(folder/'index.json',new)
 check({k:v for k,v in old.items() if k!='assets'}=={k:v for k,v in new.items() if k!='assets'},'Index top-level drift')
 check(set(newrows)-set(oldrows)=={'croisement01-tree-01'} and not set(oldrows)-set(newrows),'Index membership drift')
 for aid,row in oldrows.items():
  if aid not in IDS: check(row==newrows[aid],'Unrelated catalog regeneration drift '+aid)
 for aid in IDS: check(not any(k in newrows[aid] for k in ['lossy_model','preview_model']),'Unexpected derivative')
 add(S/'croisement01.rhlos-map.json',L/'scenes/croisement01.rhlos-map.json');add(folder/'index.json',A/'index.json')
 check(core.sha(A/'index.json')==index_pin['sha256'],'Index drift while preparing')
 protected=[]
 for row in newrows.values():
  if row['id'] not in IDS: protected.append(pin(A/row['descriptor']))
 document=json.loads((S/'croisement01.rhlos-map.json').read_text())
 for ref in document['assetSources']+document.get('sceneAssets',[]):
  for resource in ref.get('resources',[]):
   p=L/resource['path'];check(core.sha(p)==resource['sha256'],'Shared resource drift');protected.append(pin(p))
  if ref['id'] not in IDS:
   for key in ['model','descriptor']:
    if key in ref: p=L/ref[key];check(core.sha(p)==ref[key+'_sha256'],'Unrelated saved source stale');protected.append(pin(p))
 # Existing proven private library resolves exact approved model/resource pins.
 strict(folder,B/'library')
 snapshot=json.loads((B/'source-snapshot.json').read_text());drift=[{'path':p,'frozen':v,'current':core.sha(R/p)} for p,v in snapshot.items() if core.sha(R/p)!=v]
 plan={'status':'PREPARED_NOT_APPLIED','records':records,'protected':protected,'evidence':[pin(B/'handoff.json'),pin(CORE),pin(Path(__file__)),pin(D/'validate.mjs'),pin(R/'level-editor/refinement/asset_index.py')], 'preservation':preservation,'unrelated_index_rows_preserved':len(oldrows)-1,'catalog_policy':'Root-authorized scoped source-of-truth generation via write_asset_index(descriptors=two IDs), exact preservation of all unrelated live rows, complete validate_asset_index before atomic write. No unrelated descriptor regeneration.', 'source_only_policy':'Supported original-model path; no lossy or preview receipts required. Exact original payloads exercised by frozen browser proof. No derivative generation or synthetic receipts.','live_runtime_drift':drift,'runtime_limit':'Frozen-source proof remains valid only for those frozen sources. Fresh live browser verification required after install; no parity certification.','rollback':'Existing shared publisher restore refuses any third-party target drift; root must use same journal. Per-file atomic replaces, resources first, map then index last; quiesce readers.'}
 dump(folder/'plan.json',plan);dump(folder/'dry-run.json',{'status':'PASS_NOT_APPLIED','plan':pin(folder/'plan.json'),'targets':len(records),'reserve_bytes':sum(Path(r['source']).stat().st_size+(Path(r['target']).stat().st_size if Path(r['target']).exists() else 0) for r in records)})
 return plan

def main():
 p=argparse.ArgumentParser();p.add_argument('--output',type=Path,required=True);p.add_argument('--apply',action='store_true');p.add_argument('--root-reviewed',action='store_true');p.add_argument('--readers-quiescent',action='store_true');p.add_argument('--rollback',action='store_true');a=p.parse_args();folder=a.output.resolve();check(folder.parent==D,'Output outside owned directory')
 with core.locked():
  if a.rollback:
   planpath=folder/'plan.json';check(core.sha(planpath)==json.loads((folder/'dry-run.json').read_text())['plan']['sha256'],'Plan drift');plan=json.loads(planpath.read_text());j=json.loads((folder/'transaction/journal.json').read_text());check(j['plan_sha256']==core.sha(planpath),'Journal scope drift');check([(r['target'],r['old_sha256'],r['new_sha256']) for r in j['files']]==[(r['target'],r['old_sha256'],r['new_sha256']) for r in plan['records']],'Rollback scope mismatch');core.restore(j['files'],folder/'transaction',j);return
  check(not folder.exists(),'Fresh output required');check(not a.apply or (a.root_reviewed and a.readers_quiescent),'Explicit root review and quiescent readers required')
  plan=prepare(folder);records=plan['records'];core.check_current(records)
  if a.apply:
   check(shutil.disk_usage(D).free>json.loads((folder/'dry-run.json').read_text())['reserve_bytes']+2**30,'Insufficient reserve')
   core.PLAN_SHA=core.sha(folder/'plan.json')
   def precommit():
    evidence();check(core.sha(folder/'plan.json')==core.PLAN_SHA,'Plan drift')
    for pin_ in plan['protected']+plan['evidence']:core.verify_pin(pin_)
   def postcheck():
    for r in records:check(core.sha(Path(r['target']))==r['new_sha256'],'Installed mismatch')
    for pin_ in plan['protected']:core.verify_pin(pin_)
    strict(folder,L)
   precommit()
   for r in records:Path(r['target']).parent.mkdir(parents=True,exist_ok=True)
   core.commit(records,folder/'transaction',postcheck,precommit=precommit)
  print(json.dumps({'status':'INSTALLED_RUNTIME_PENDING' if a.apply else 'PREPARED_NOT_APPLIED','directory':str(folder),'targets':len(records),'runtime_drift':len(plan['live_runtime_drift'])}))
if __name__=='__main__':main()
