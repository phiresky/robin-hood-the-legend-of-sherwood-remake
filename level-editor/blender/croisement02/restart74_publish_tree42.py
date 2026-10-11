"""Root-only additive tree42 publication, with a fresh catalog rebase and rollback."""
import argparse,json,shutil,subprocess
from pathlib import Path
import restart58_publish_holes_mounds as shared
io=shared.io
ROOT,LIB,BASE=shared.ROOT,shared.LIB,shared.BASE
STAGE=BASE/'restart62-tree42-catalog-stage-v2'
RUNS=BASE/'restart74-tree42-publication-v1'
STAGE_SHA='2f65c7edd029282ca7201ca59999141c2deabbfa9d63d446801c8bf2efff76c6'
sha,require,dump=io.sha,io.require,io.dump
read=shared.read
SELF_SHA=sha(Path(__file__))
def proof(path,digest):
 require(path and sha(path)==digest,'Pinned tree42 normal editor proof required')
 doc=read(path)
 require(doc.get('status')=='PASS_TREE42_CATALOG_UI' and doc.get('stage_sha256')==STAGE_SHA,'Wrong UI proof scope')
 require(set(doc.get('entry_ids',[]))=={e['id']for e in read(STAGE/'plan.json')['catalog_additions']},'Wrong proven entries')
 require(bool(doc.get('evidence')) and bool(doc.get('runtime')),'Proof evidence/runtime pins absent')
 for p,h in {**doc['evidence'],**doc['runtime']}.items():require(sha(ROOT/p)==h,'Proof pin drift '+p)
 return io.pin(path)
def stage():
 require(sha(STAGE/'plan.json')==STAGE_SHA,'Stage drift')
 d=read(STAGE/'plan.json')
 for p in [d['approval'],d['export_readback']]:io.core.verify_pin(p)
 for r in d['files']:io.core.verify_pin(r['staged'])
 return d
def prepare(folder,browser=None,digest=None):
 d=stage();current=read(LIB/'mission-states/index.json');before=sha(LIB/'mission-states/index.json');ids=[e['id']for e in current['entries']]
 require(len(ids)==len(set(ids)) and not set(ids)&{e['id']for e in d['catalog_additions']},'Duplicate or installed tree42 entry')
 folder.mkdir(exist_ok=False)
 subprocess.run(['node',str(Path(__file__).with_name('restart74_verify_tree42_catalog.mjs'))],cwd=ROOT,check=True)
 rows=[]
 for r in d['files']:
  rel=Path(r['destination'])
  if str(rel)=='mission-states/index.json':continue
  require(rel.is_relative_to('mission-states/croisement02/reviewed-tree42-motion-v1')and '..'not in rel.parts,'Out of scope target')
  target=LIB/rel;require(target.parent.resolve()==target.parent and not target.is_symlink(),'Symlink target');old=sha(target);new=r['staged']['sha256'];require(old in [None,new],'Conflicting immutable resource')
  if old!=new:rows.append({'source':r['staged']['path'],'target':str(target),'old_sha256':old,'new_sha256':new})
 merged={**current,'entries':[*current['entries'],*d['catalog_additions']]};dump(folder/'index.json',merged);rows.append({'source':str(folder/'index.json'),'target':str(LIB/'mission-states/index.json'),'old_sha256':before,'new_sha256':sha(folder/'index.json')})
 ui=proof(browser,digest)if browser else None
 require(sha(LIB/'mission-states/index.json')==before,'Concurrent catalog drift')
 p={'status':'PREPARED_NOT_APPLIED','records':rows,'catalog_before':current,'additions':d['catalog_additions'],'browser':ui,'stage':io.pin(STAGE/'plan.json'),'verifier':io.pin(Path(__file__).with_name('restart74_verify_tree42_catalog.mjs')),'executor':io.pin(Path(__file__).resolve()),'map_guard':io.pin(LIB/'scenes/croisement02.rhlos-map.json'),'limitations':['Tree42 S03 loop only; other mission bindings and whole-map gameplay compilation are separate.']};dump(folder/'plan.json',p);dump(folder/'dry-run.json',{'status':'PASS_NOT_APPLIED','plan':io.pin(folder/'plan.json'),'targets':len(rows),'unrelated_entries_preserved':len(ids),'catalog_ui_verified':ui is not None});return p

def main():
 p=argparse.ArgumentParser();p.add_argument('--output',type=Path,required=True);p.add_argument('--apply',action='store_true');p.add_argument('--root-reviewed',action='store_true');p.add_argument('--readers-quiescent',action='store_true');p.add_argument('--browser-proof',type=Path);p.add_argument('--browser-sha256');a=p.parse_args();folder=a.output.resolve();require(folder.parent==RUNS,'Wrong output directory');RUNS.mkdir(exist_ok=True)
 require(not a.apply or(a.root_reviewed and a.readers_quiescent and a.browser_proof),'Root review, quiescent readers and UI proof required')
 with io.core.locked():
  plan=prepare(folder,a.browser_proof,a.browser_sha256);rows=plan['records'];digest=sha(folder/'plan.json')
  if a.apply:
   require(shutil.disk_usage(RUNS).free>sum(Path(r['source']).stat().st_size+(Path(r['target']).stat().st_size if Path(r['target']).exists()else 0)for r in rows)+2**30,'Rollback reserve insufficient')
   def precommit():
    stage();proof(a.browser_proof,a.browser_sha256);require(sha(Path(__file__))==SELF_SHA and sha(folder/'plan.json')==digest,'Executor/plan drift');io.core.verify_pin(plan['verifier']);io.core.verify_pin(plan['map_guard'])
    for r in rows:require(sha(Path(r['source']))==r['new_sha256'],'Payload drift')
   precommit()
   for r in rows:Path(r['target']).parent.mkdir(parents=True,exist_ok=True)
   def postcheck():
    io.verify(rows,'new_sha256');require(read(LIB/'mission-states/index.json')=={**plan['catalog_before'],'entries':[*plan['catalog_before']['entries'],*plan['additions']]},'Merged catalog differs')
   io.commit(rows,folder/'transaction',digest,precommit,postcheck)
 print(json.dumps({'status':'INSTALLED_LIVE_BROWSER_PENDING'if a.apply else'PREPARED_NOT_APPLIED','output':str(folder)}))
if __name__=='__main__':main()
