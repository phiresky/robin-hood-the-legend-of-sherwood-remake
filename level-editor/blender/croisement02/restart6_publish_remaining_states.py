"""Guarded state-only transaction; dry-run unless explicitly selected by shared publisher."""
from pathlib import Path
import argparse,fcntl,hashlib,json,os,tempfile
ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'level-editor/work/croisement02-refinement/restart2-state'
PLAN=BASE/'remaining-seven-publication-v1'
STAGE=BASE/'remaining-seven-package-v2'
LIB=ROOT/'level-editor/library'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def read(p):return json.loads(p.read_text())
def checked(base,relative):
 p=Path(relative)
 if p.is_absolute() or '..' in p.parts:raise ValueError('Unsafe relative path')
 return base/p

def evidence(pin):
 p=checked(ROOT,pin['path']);assert sha(p)==pin['sha256'],str(p);return read(p)

def replace_index(data,expected):
 index=LIB/'mission-states/index.json';assert sha(index)==expected,'Concurrent catalog change'
 fd,name=tempfile.mkstemp(prefix='.remaining-seven-',suffix='.json',dir=index.parent)
 with os.fdopen(fd,'wb') as f:f.write(data);f.flush();os.fsync(f.fileno())
 assert sha(index)==expected,'Concurrent catalog change before atomic switch'
 os.replace(name,index)
 directory=os.open(index.parent,os.O_RDONLY)
 try:os.fsync(directory)
 finally:os.close(directory)

def main():
 ap=argparse.ArgumentParser(description=__doc__);group=ap.add_mutually_exclusive_group();group.add_argument('--install',action='store_true');group.add_argument('--rollback',action='store_true');ap.add_argument('--gate',type=Path);a=ap.parse_args()
 plan=read(PLAN/'plan.json');manifest=evidence(plan['manifest']);index=LIB/'mission-states/index.json';old=plan['installed_index_sha256'];new=manifest['private_index']['sha256']
 assert sha(PLAN/'installed34-index.json')==old
 if a.rollback:
  with (PLAN/'transaction.lock').open('a') as lock:
   fcntl.flock(lock,fcntl.LOCK_EX);assert sha(index)==new,'Rollback refuses a different current catalog'
   for path,h in plan['baseline_files'].items():
    if path!='mission-states/index.json':assert sha(checked(LIB,path))==h
   replace_index((PLAN/'installed34-index.json').read_bytes(),new)
   receipt={'status':'ROLLED_BACK_INDEX_ONLY','restored_sha256':sha(index),'immutable_resources_retained':True};(PLAN/'rollback.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps(receipt));return
 assert sha(index)==old,'Existing catalog no longer matches frozen34 baseline'
 for row in manifest['files']+[manifest['private_index']]:assert sha(checked(STAGE/'library',row['path']))==row['sha256']
 for path,h in plan['baseline_files'].items():assert sha(checked(LIB,path))==h
 for row in manifest['reused']:assert sha(checked(LIB,row['path']))==row['sha256']
 if not a.install:
  print(json.dumps({'status':'DRY_RUN_PASS','files':len(manifest['files']),'live_entries':34,'private_entries':41,'gate_pending':True,'library_modified':False}));return
 assert a.gate,'--install requires successful final proof/publication gate'
 gate=read(a.gate);assert gate['status']=='PASS_READY_FOR_SHARED_PUBLISHER';assert gate['plan_sha256']==sha(PLAN/'plan.json')
 proof=evidence(gate['state_proof']);assert proof['status']=='PASS' and proof['manifest_sha256']==plan['manifest']['sha256'];assert proof['checks'] and all(c['pass'] for c in proof['checks'])
 for id in plan['added_ids']:assert any(c['name']==id+' exact contract' and c['pass'] for c in proof['checks']),id
 review=evidence(gate['root_review']);assert 'PASS' in review['status'];assert review['verification_sha256']==gate['state_proof']['sha256']
 publication=evidence(gate['static_publication']);assert 'PASS' in publication['status']
 bindings=evidence(gate['static_binding_verification']);assert bindings['status']=='PASS' and bindings['contract_manifest_sha256']==plan['manifest']['sha256']
 final=gate['final_static'];assert bindings['map_sha256']==final['map_sha256'];assert final['map_path']=='scenes/croisement02.rhlos-map.json'
 assert sha(checked(LIB,final['map_path']))==final['map_sha256']
 for row in final['source_resources']:assert sha(checked(LIB,row['path']))==row['sha256']
 assert final['source_resources'] and gate['runtime_pins']
 for path,h in gate['runtime_pins'].items():assert sha(checked(ROOT,path))==h
 # Review must explicitly reconcile the tested fixture with any final approved static delta.
 if final['map_sha256']!=plan['static_dependency']['map']['sha256']:
  compatibility=evidence(gate['static_delta_review']);assert 'PASS' in compatibility['status'];assert compatibility['final_map_sha256']==final['map_sha256'] and compatibility['tested_map_sha256']==plan['static_dependency']['map']['sha256']
 with (PLAN/'transaction.lock').open('a') as lock:
  fcntl.flock(lock,fcntl.LOCK_EX);assert sha(index)==old
  for row in manifest['files']:
   assert row['path'].startswith('mission-states/') and row['path']!='mission-states/index.json'
   source=checked(STAGE/'library',row['path']);target=checked(LIB,row['path']);assert sha(source)==row['sha256'];target.parent.mkdir(parents=True,exist_ok=True)
   if not target.exists():
    with target.open('xb') as f:f.write(source.read_bytes());f.flush();os.fsync(f.fileno())
   assert sha(target)==row['sha256']
  for row in manifest['files']+manifest['reused']:assert sha(checked(LIB,row['path']))==row['sha256']
  for path,h in plan['baseline_files'].items():assert sha(checked(LIB,path))==h
  assert sha(checked(LIB,final['map_path']))==final['map_sha256']
  replace_index((STAGE/'library'/manifest['private_index']['path']).read_bytes(),old)
  receipt={'status':'INSTALLED_PENDING_NORMAL_HTTP_PROOF','entries_added':7,'index_sha256':sha(index),'baseline_index_sha256':old,'baseline_nonindex_preserved':len(plan['baseline_files'])-1,'plan_sha256':sha(PLAN/'plan.json'),'gate_sha256':sha(a.gate),'final_static_map_sha256':final['map_sha256'],'rollback':'Restore only frozen34 index if currentindex still exactly this41; keep all immutable resources.'}
  (PLAN/'installation.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps(receipt))
if __name__=='__main__':main()
