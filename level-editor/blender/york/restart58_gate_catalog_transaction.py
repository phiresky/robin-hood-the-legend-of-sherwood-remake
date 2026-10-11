"""Root-only gate catalog transaction; preserve unrelated entries and switch catalog last."""
import argparse,json,shutil
from pathlib import Path
import restart44_static_transaction as io
ROOT,LIB,BASE=io.ROOT,io.LIB,io.WORK
STAGE=BASE/'restart57-gate-catalog-stage-v1'
RUNS=BASE/'restart58-gate-catalog-transaction-v1'
STAGE_SHA='efdca69ef32358cf80deadb424aec29a8e782c96ad69c0920a7d35fc32f1f438'
HELPER_SHA='a71796d93dbfbc8fd07ebd6deebf8bc0177e7e599a85b9233c43147fd38a30e4'
sha,require,dump=io.sha,io.require,io.dump
SELF_SHA=sha(Path(__file__))
def read(p):return json.loads(p.read_text())
def proof(path,digest):
 require(path and sha(path)==digest,'Exact UI proof required')
 d=read(path);require(d.get('status')=='PASS' and d.get('entry_id')=='york-portcullis','Wrong proof scope')
 require(d.get('contract_sha256')==read(STAGE/'plan.json')['entry']['contract']['sha256'],'Wrong proven contract')
 require(d.get('editor_checks')==dict.fromkeys(['map_load','catalog_selection','physical_motion_play','seek_terminal','reset','native_mode','map_only_cleanup'],True),'Incomplete normal editor proof')
 require(bool(d.get('evidence')),'Missing evidence pins')
 for p,h in d['evidence'].items():require(sha(Path(p))==h,'Proof evidence drift')
 return io.pin(path)
def prepare(folder,proof_path=None,proof_sha=None):
 require(sha(STAGE/'plan.json')==STAGE_SHA,'Stage drift');require(sha(Path(io.__file__))==HELPER_SHA,'Transaction helper drift')
 stage=read(STAGE/'plan.json');rows=[]
 for r in stage['records']:
  rel=Path(r['relative']);source=Path(r['source']);target=LIB/rel
  require(rel.is_relative_to('mission-states/york') or str(rel)=='mission-states/index.json','Out-of-scope payload')
  require(source.resolve()==source and target.parent.resolve()==target.parent and not target.is_symlink(),'Symlink publication path')
  require(sha(source)==r['sha256'],'Stage payload drift')
  if str(rel)=='mission-states/index.json':continue
  old=sha(target);require(old in [None,r['sha256']],'Conflicting immutable gate resource')
  if old!=r['sha256']:rows.append({'source':str(source),'target':str(target),'old_sha256':old,'new_sha256':r['sha256']})
 index=LIB/'mission-states/index.json';before=read(index);old_index=sha(index)
 require(not any(r['id']==stage['entry']['id'] for r in before['entries']),'Gate entry already installed; reconcile receipt')
 after={**before,'entries':[*before['entries'],stage['entry']]}
 folder.mkdir(exist_ok=False);dump(folder/'index.json',after)
 rows.append({'source':str(folder/'index.json'),'target':str(index),'old_sha256':old_index,'new_sha256':sha(folder/'index.json')})
 ui=proof(proof_path,proof_sha) if proof_path else None
 result={'status':'PREPARED_NOT_APPLIED','records':rows,'stage':io.pin(STAGE/'plan.json'),'executor':io.pin(Path(__file__).resolve()),'helper':io.pin(Path(io.__file__).resolve()),'browser':ui,'catalog_before':before,'entry':stage['entry'],'scope':'Only new immutable York mission resources plus one catalog entry; current unrelated catalog rows/order/metadata preserved. Catalog switched last. No map or global asset index writes.'}
 require(sha(index)==old_index,'Catalog drift while preparing');dump(folder/'plan.json',result);dump(folder/'dry-run.json',{'status':'PASS_NOT_APPLIED','plan':io.pin(folder/'plan.json'),'targets':len(rows),'normal_editor_verified':ui is not None});return result

def main():
 p=argparse.ArgumentParser();p.add_argument('--output',type=Path,required=True);p.add_argument('--apply',action='store_true');p.add_argument('--rollback',action='store_true');p.add_argument('--root-reviewed',action='store_true');p.add_argument('--readers-quiescent',action='store_true');p.add_argument('--browser-proof',type=Path);p.add_argument('--browser-sha256');a=p.parse_args();folder=a.output.resolve()
 require(folder.parent==RUNS,'Output outside owned directory');RUNS.mkdir(exist_ok=True)
 with io.core.locked():
  if a.rollback:
   plan=read(folder/'plan.json');journal=read(folder/'transaction/journal.json');require(sha(folder/'plan.json')==read(folder/'dry-run.json')['plan']['sha256']==journal['plan_sha256'],'Rollback plan drift')
   require([(r['target'],r['old_sha256'],r['new_sha256']) for r in plan['records']]==[(r['target'],r['old_sha256'],r['new_sha256']) for r in journal['files']],'Rollback target drift')
   io.core.restore(journal['files'],folder/'transaction',journal);return
  require(not a.apply or (a.root_reviewed and a.readers_quiescent and a.browser_proof),'Root review/quiescent readers/proof required')
  plan=prepare(folder,a.browser_proof,a.browser_sha256);rows=plan['records'];plan_hash=sha(folder/'plan.json')
  if a.apply:
   reserve=sum(Path(r['source']).stat().st_size+(Path(r['target']).stat().st_size if Path(r['target']).exists() else 0) for r in rows);require(shutil.disk_usage(RUNS).free>reserve+2**30,'Insufficient rollback reserve')
   def check():
    require(sha(Path(__file__))==SELF_SHA and sha(Path(io.__file__))==HELPER_SHA,'Executor drift');require(sha(folder/'plan.json')==plan_hash and sha(STAGE/'plan.json')==STAGE_SHA,'Plan drift');proof(a.browser_proof,a.browser_sha256)
    for r in rows:
     require(sha(Path(r['source']))==r['new_sha256'],'Source drift')
     target=Path(r['target']);require(target.parent.resolve()==target.parent and not target.is_symlink(),'Target path changed')
   check()
   for r in rows:Path(r['target']).parent.mkdir(parents=True,exist_ok=True)
   def postcheck():
    io.verify(rows,'new_sha256');live=read(LIB/'mission-states/index.json');require(live=={**plan['catalog_before'],'entries':[*plan['catalog_before']['entries'],plan['entry']]},'Live catalog semantic drift')
    for r in read(STAGE/'plan.json')['records']:
     if r['relative']!='mission-states/index.json':require(sha(LIB/r['relative'])==r['sha256'],'Installed resource drift')
   io.commit(rows,folder/'transaction',plan_hash,check,postcheck)
  print(json.dumps({'status':'INSTALLED_LIVE_UI_PENDING' if a.apply else 'PREPARED_NOT_APPLIED','output':str(folder),'targets':len(rows)}))
if __name__=='__main__':main()
