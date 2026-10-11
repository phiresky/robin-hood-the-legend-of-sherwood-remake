"""Prepare or apply a root-authorized, narrowly scoped recovered-state transaction."""
import argparse,difflib,json,shutil,subprocess
from pathlib import Path
import restart60_publish_climbing as climb
import restart74_publish_tree42 as tree
io=climb.io
ROOT,LIB,BASE=climb.ROOT,climb.LIB,climb.BASE
sha,require,dump=io.sha,io.require,io.dump
read=climb.read
REVIEW=BASE/'restart79-climbing-tree42-ui-browser-v1'
RUNS=BASE/'restart113-climbing-tree42-publication-v1'
RECOVERY_SHA='68a8cfa71184f9884c89ddc758e923b475a62d87b725d03b791ed0ee5105c951'
SELF=Path(__file__).resolve()

def verified_recovery():
 recovery=REVIEW/'recovered-review.json';require(sha(recovery)==RECOVERY_SHA,'Recovery changed')
 doc=read(recovery);root=read(REVIEW/'root-recovered-evidence-review.json')
 require(root['status']=='ACCEPT_SCOPED_RECOVERED_UI_EVIDENCE' and root['review_sha256']==RECOVERY_SHA,'Missing root recovered evidence decision')
 require(doc['original_ui_run']['functional_checks']==84 and doc['original_ui_run']['exit_code']==143,'Unexpected recovered scope')
 for p,h in doc['evidence'].items():require(sha(ROOT/p)==h,'Recovery evidence changed '+p)
 return doc

def current_runtime(doc):
 paths=set(doc['runtime'])
 for directory in [ROOT/'level-editor/app/src',ROOT/'level-editor/shared/src']:
  for p in directory.rglob('*'):
   if p.is_file() and not p.is_symlink() and '__pycache__' not in p.parts:paths.add(str(p.relative_to(ROOT)))
 return {p:sha(ROOT/p) for p in sorted(paths)}

def prepare(folder):
 doc=verified_recovery();stages=[climb.stage(),tree.stage()];folder.mkdir(exist_ok=False)
 before=read(LIB/'mission-states/index.json');index_sha=sha(LIB/'mission-states/index.json');additions=[e for s in stages for e in s['catalog_additions']]
 require(len(additions)==3 and len({e['id'] for e in additions})==3,'Wrong additions')
 ids=[e['id'] for e in before['entries']];require(len(ids)==len(set(ids)) and not set(ids)&{e['id'] for e in additions},'Duplicate/installed entries')
 checks=[]
 for verifier in ['restart59_verify_climbing_catalog.mjs','restart74_verify_tree42_catalog.mjs']:
  p=SELF.with_name(verifier);r=subprocess.run(['node',str(p)],cwd=ROOT,text=True,capture_output=True)
  dump(folder/(verifier+'.result.json'),dict(exit_code=r.returncode,stdout=r.stdout,stderr=r.stderr));require(r.returncode==0,'Current verifier failed '+verifier);checks.append(io.pin(p))
 runtime=current_runtime(doc);drifts=[];(folder/'diffs').mkdir()
 for p,old in doc['runtime'].items():
  new=runtime[p]
  if new==old:continue
  frozen=REVIEW/'snapshot'/p;require(sha(frozen)==old,'Frozen runtime missing '+p)
  diff=''.join(difflib.unified_diff(frozen.read_text().splitlines(True),(ROOT/p).read_text().splitlines(True),fromfile='tested/'+p,tofile='current/'+p))
  dest=folder/'diffs'/p.replace('/','__');dest.write_text(diff)
  relevant=p in ['level-editor/app/src/fs.ts','level-editor/app/src/http-library.ts']
  drifts.append(dict(path=p,tested_sha256=old,current_sha256=new,diff=io.pin(dest),relevance='HTTP library/catalog read path changed; current code NOT certified by recovered UI. Root must explicitly accept staged installation plus mandatory current-runtime postinstall verification.' if relevant else 'Gameplay compiler/navigation change outside this catalog/state-preview proof. Current bytes remain strictly guarded; no compiler or engine-parity certification.'))
 records=[];allowed=['mission-states/croisement02/reviewed-climbing-v1','mission-states/croisement02/reviewed-tree42-motion-v1']
 for stage in stages:
  for row in stage['files']:
   rel=Path(row['destination'])
   if str(rel)=='mission-states/index.json':continue
   require('..' not in rel.parts and any(rel.is_relative_to(a) for a in allowed),'Out-of-scope resource')
   target=LIB/rel;require(not target.is_symlink() and target.parent.resolve()==target.parent,'Symlink target')
   old=sha(target);new=row['staged']['sha256'];require(old in [None,new],'Conflicting immutable resource')
   if old!=new:records.append(dict(source=row['staged']['path'],target=str(target),old_sha256=old,new_sha256=new))
 merged={**before,'entries':[*before['entries'],*additions]};dump(folder/'index.json',merged)
 records.append(dict(source=str(folder/'index.json'),target=str(LIB/'mission-states/index.json'),old_sha256=index_sha,new_sha256=sha(folder/'index.json')))
 require(len({r['target'] for r in records})==len(records),'Duplicate transaction target')
 require(sha(LIB/'mission-states/index.json')==index_sha,'Concurrent catalog drift')
 evidence=[io.pin(REVIEW/'root-recovered-evidence-review.json'),io.pin(REVIEW/'recovered-review.json'),io.pin(REVIEW/'climbing-recovered-review.json'),io.pin(REVIEW/'tree42-recovered-review.json')]
 proposal=dict(status='PREPARED_NOT_APPLIED_REQUIRES_EXACT_ROOT_AUTHORITY',executor=io.pin(SELF),executor_dependencies=[io.pin(Path(m.__file__).resolve()) for m in [climb,tree,climb.shared,io,io.core]],records=records,catalog_before=before,additions=additions,stages=[io.pin(climb.STAGE/'plan.json'),io.pin(tree.STAGE/'plan.json')],recovery=evidence,verifiers=checks,map_guard=io.pin(LIB/'scenes/croisement02.rhlos-map.json'),runtime_current=runtime,runtime_tested=doc['runtime'],runtime_drift=drifts,new_runtime_paths=sorted(set(runtime)-set(doc['runtime'])),limitations=['Recovered 84 checks and four captures, original exit143; original library response log unavailable. Root accepted scoped inference, not a clean fullrun.','Changed HTTP JSON/catalog path is not proven by recovered frozen UI; exact current bytes guarded pending mandatory postinstall UI.','No gameplay/compiler, remaining native loops, whole-map completion or newer geometry approval claim.'],postinstall_required=True)
 dump(folder/'plan.json',proposal);dump(folder/'summary.json',dict(status=proposal['status'],plan=io.pin(folder/'plan.json'),preserved_entries=len(ids),added_entries=len(additions),result_entries=len(merged['entries']),targets=len(records),changed_runtime_paths=len(drifts),current_runtime_guards=len(runtime),new_runtime_paths=proposal['new_runtime_paths']))
 return proposal

def validate(plan,plan_path,authority):
 require(authority['status']=='AUTHORIZE_SCOPED_CLIMBING_TREE42_INSTALL','Wrong root authority')
 require(authority['plan_sha256']==sha(plan_path),'Root authority must bind exact plan')
 require(authority.get('accepts_recovered_ui_limitations') is True and authority.get('requires_postinstall_current_runtime_ui') is True,'Missing scoped provenance conditions')
 require(authority.get('accepted_runtime_drift_paths')==[r['path'] for r in plan['runtime_drift']],'Root must explicitly review every runtime drift')
 verified_recovery();climb.stage();tree.stage()
 for pin in [plan['executor'],*plan['executor_dependencies'],plan['map_guard'],*plan['stages'],*plan['recovery'],*plan['verifiers']]:io.core.verify_pin(pin)
 for row in plan['runtime_drift']:io.core.verify_pin(row['diff'])
 for p,h in plan['runtime_current'].items():require(sha(ROOT/p)==h,'Current runtime drift '+p)
 require(current_runtime({'runtime':plan['runtime_tested']})==plan['runtime_current'],'New runtime files since proposal')
 for r in plan['records']:require(sha(Path(r['source']))==r['new_sha256'],'Payload changed')
 io.verify(plan['records'],'old_sha256')

def main():
 p=argparse.ArgumentParser();p.add_argument('--output',type=Path,required=True);p.add_argument('--apply-plan',type=Path);p.add_argument('--root-authority',type=Path);p.add_argument('--root-authority-sha256');p.add_argument('--readers-quiescent',action='store_true');a=p.parse_args();folder=a.output.resolve();RUNS.mkdir(exist_ok=True);require(folder.parent==RUNS,'Output outside transaction workspace')
 with io.core.locked():
  if not a.apply_plan:prepare(folder)
  else:
   require(a.root_authority and a.readers_quiescent and sha(a.root_authority)==a.root_authority_sha256,'Pinned root authority and quiescent readers required')
   plan_path=a.apply_plan.resolve();require(plan_path.parent.parent==RUNS,'Plan outside transaction workspace');plan=read(plan_path);authority=read(a.root_authority);digest=sha(plan_path)
   validate(plan,plan_path,authority);folder.mkdir(exist_ok=False)
   rows=plan['records'];require(shutil.disk_usage(RUNS).free>sum(Path(r['source']).stat().st_size+(Path(r['target']).stat().st_size if Path(r['target']).exists() else 0) for r in rows)+2**30,'Rollback reserve insufficient')
   def precommit():
    require(sha(plan_path)==digest and sha(a.root_authority)==a.root_authority_sha256,'Plan/authority drift');validate(plan,plan_path,authority)
   def postcheck():
    io.verify(rows,'new_sha256');require(read(LIB/'mission-states/index.json')=={**plan['catalog_before'],'entries':[*plan['catalog_before']['entries'],*plan['additions']]},'Catalog merge mismatch')
   for r in rows:Path(r['target']).parent.mkdir(parents=True,exist_ok=True)
   io.commit(rows,folder/'transaction',digest,precommit,postcheck);dump(folder/'installed.json',dict(status='INSTALLED_CURRENT_RUNTIME_POSTINSTALL_UI_PENDING',plan=io.pin(plan_path),authority=io.pin(a.root_authority)))
 print(json.dumps(dict(status='INSTALLED_CURRENT_RUNTIME_POSTINSTALL_UI_PENDING' if a.apply_plan else 'PREPARED_NOT_APPLIED',output=str(folder))))
if __name__=='__main__':main()
