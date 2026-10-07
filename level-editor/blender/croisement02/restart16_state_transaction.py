"""Publish the frozen seven-state package; default mode only verifies bytes."""
from pathlib import Path
import argparse,fcntl,hashlib,json,os,tempfile
ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'level-editor/work/croisement02-refinement/restart2-state'
PLAN=BASE/'remaining-seven-publication-v1'
STAGE=BASE/'remaining-seven-package-v2/library'
LIB=ROOT/'level-editor/library'
PLAN_SHA='e3036288cc9eb40e726be6c63027d23ba62a17a0f096c784053e933e511e4dce'
INDEX='mission-states/index.json'

def require(condition,message):
 if not condition:raise ValueError(message)
def sha(path):
 h=hashlib.sha256()
 with Path(path).open('rb')as f:
  for block in iter(lambda:f.read(1024*1024),b''):h.update(block)
 return h.hexdigest()
def read(path):return json.loads(Path(path).read_text())
def checked(base,relative):
 base=Path(base).resolve();rel=Path(relative)
 require(not rel.is_absolute()and rel.parts and '..'not in rel.parts,'Unsafe relative path')
 path=base/rel
 require(path.resolve().is_relative_to(base),'Path escapes root')
 for component in [path,*path.parents]:
  if component==base:break
  require(not component.is_symlink(),'Symlink destination rejected')
 return path

def evidence(pin):
 path=checked(ROOT,pin['path']);require(sha(path)==pin['sha256'],f'Evidence drift: {path}');return read(path)
def fsync_dir(path):
 fd=os.open(path,os.O_RDONLY)
 try:os.fsync(fd)
 finally:os.close(fd)
def atomic_absent(target,data,expected):
 """Link a fully flushed temporary inode without replacing an existing destination."""
 require(hashlib.sha256(data).hexdigest()==expected,'Input bytes differ')
 if target.exists():require(sha(target)==expected,'Existing immutable destination differs');return False
 target.parent.mkdir(parents=True,exist_ok=True)
 fd,name=tempfile.mkstemp(prefix='.state-transaction-',dir=target.parent)
 try:
  with os.fdopen(fd,'wb')as f:f.write(data);f.flush();os.fsync(f.fileno())
  try:os.link(name,target)
  except FileExistsError:require(sha(target)==expected,'Concurrent immutable destination differs');return False
  fsync_dir(target.parent);return True
 finally:os.unlink(name)
def switch_index(index,data,expected,replacement_sha):
 require(hashlib.sha256(data).hexdigest()==replacement_sha,'Exact replacement index bytes drifted')
 require(sha(index)==expected,'Concurrent index change')
 fd,name=tempfile.mkstemp(prefix='.state-index-',dir=index.parent)
 try:
  with os.fdopen(fd,'wb')as f:f.write(data);f.flush();os.fsync(f.fileno())
  require(sha(index)==expected,'Concurrent index change before switch')
  os.replace(name,index);fsync_dir(index.parent)
 finally:
  if os.path.exists(name):os.unlink(name)
def mappings(rows):
 result={r['path']:r['sha256']for r in rows};require(len(result)==len(rows),'Duplicate file paths');return result

def verify_package(plan,manifest,installed=True):
 require(len(manifest['files'])==310 and len(manifest['reused'])==976,'Frozen package counts differ')
 require(mappings(plan['files'])==mappings(manifest['files'])and mappings(plan['reused'])==mappings(manifest['reused']),'Plan/manifest file mismatch')
 require(not(set(mappings(manifest['files']))&set(mappings(manifest['reused']))),'New/reused overlap')
 require(len(plan['baseline_files'])==1300,'Frozen baseline count differs')
 old=plan['installed_index_sha256'];new=manifest['private_index']['sha256']
 require(sha(PLAN/'installed34-index.json')==old==plan['baseline_index']['sha256'],'Baseline index drift')
 require(evidence(plan['staged_index'])==read(STAGE/INDEX),'Staged index mismatch')
 before=read(PLAN/'installed34-index.json')['entries'];after=read(STAGE/INDEX)['entries']
 require(len(before)==34 and len(after)==41,'Wrong entry count')
 by_id={x['id']:x for x in after};require(len(by_id)==41,'Duplicate entry IDs')
 require(all(by_id.get(x['id'])==x for x in before),'Existing entry changed')
 require(set(by_id)-{x['id']for x in before}==set(plan['added_ids']),'Added IDs differ')
 for row in manifest['files']+[manifest['private_index']]:
  require(row['path'].startswith('mission-states/'),'Nonstate destination')
  source=checked(STAGE,row['path']);require(sha(source)==row['sha256'],'Staged file drift')
  if 'bytes'in row:require(source.stat().st_size==row['bytes'],'Staged size drift')
 for row in manifest['files']:
  require(row['path']!=INDEX,'Index present among immutable files')
  dest=checked(LIB,row['path'])
  if dest.exists():require(sha(dest)==row['sha256'],'Existing new-path bytes conflict')
 for row in manifest['reused']:require(sha(checked(LIB,row['path']))==row['sha256'],'Reused file drift')
 for path,digest in plan['baseline_files'].items():
  if path!=INDEX or installed:require(sha(checked(LIB,path))==digest,'Baseline file drift')
 if installed:require(sha(LIB/INDEX)==old,'Live index no longer frozen34')
 return old,new

def verify_gate(gate_path,gate_sha,plan):
 require(gate_path is not None and gate_sha,'Explicit gate and hash required')
 require(sha(gate_path)==gate_sha,'Gate drift');gate=read(gate_path)
 require(gate['status']=='PASS_READY_FOR_SHARED_PUBLISHER'and gate['plan_sha256']==PLAN_SHA,'Gate not ready for exact plan')
 proof=evidence(gate['state_proof']);require(proof['status']=='PASS'and proof['manifest_sha256']==plan['manifest']['sha256'],'State proof mismatch')
 require(proof['checks']and all(c['pass']for c in proof['checks']),'Failed state assertions')
 for identifier in plan['added_ids']:require(any(c['name']==identifier+' exact contract'and c['pass']for c in proof['checks']),'Missing exact contract check')
 review=evidence(gate['root_review']);require('PASS'in review['status']and review.get('verification_sha256',review.get('state_verification_sha256'))==gate['state_proof']['sha256'],'Root review mismatch')
 require('PASS'in evidence(gate['static_publication'])['status'],'Static publication not verified')
 bindings=evidence(gate['static_binding_verification']);final=gate['final_static']
 require(bindings['status']=='PASS'and bindings['contract_manifest_sha256']==plan['manifest']['sha256']and bindings['map_sha256']==final['map_sha256'],'Static bindings mismatch')
 require(final['map_path']=='scenes/croisement02.rhlos-map.json'and sha(checked(LIB,final['map_path']))==final['map_sha256'],'Current static map differs')
 require(final['source_resources']and gate['runtime_pins'],'Missing resource/runtime pins')
 for row in final['source_resources']:require(sha(checked(LIB,row['path']))==row['sha256'],'Current static resource differs')
 for path,digest in gate['runtime_pins'].items():require(sha(checked(ROOT,path))==digest,'Runtime changed')
 if final['map_sha256']!=plan['static_dependency']['map']['sha256']:
  delta=evidence(gate['static_delta_review']);require('PASS'in delta['status']and delta['final_map_sha256']==final['map_sha256']and delta['tested_map_sha256']==plan['static_dependency']['map']['sha256'],'Static delta lacks review')
 normal=evidence(gate['static_normal_http']);require(normal['status']=='PASS'and normal['map_sha256']==final['map_sha256'],'Normal-library proof mismatch')
 return gate

def rollback_owned(index,baseline,new,installation,old):
 require(hashlib.sha256(baseline).hexdigest()==old,'Pinned rollback baseline bytes differ')
 require(installation['baseline_index_sha256']==old,'Installation baseline binding differs')
 require(installation['status']in ['INSTALLED_PENDING_NORMAL_HTTP_PROOF','TRANSACTION_PREPARED','INDEX_SWITCHED_PENDING_RECOVERY']and installation['plan_sha256']==PLAN_SHA and installation['index_sha256']==new,'No matching owned installation receipt')
 require(sha(index)==new,'Rollback refuses another current index')
 switch_index(index,baseline,new,old)

def main():
 ap=argparse.ArgumentParser(description=__doc__);mode=ap.add_mutually_exclusive_group();mode.add_argument('--execute',action='store_true');mode.add_argument('--rollback',action='store_true');ap.add_argument('--plan-sha',default=PLAN_SHA);ap.add_argument('--gate',type=Path);ap.add_argument('--gate-sha');ap.add_argument('--receipt',type=Path);ap.add_argument('--installation',type=Path);ap.add_argument('--installation-sha');args=ap.parse_args()
 require(args.plan_sha==PLAN_SHA==sha(PLAN/'plan.json'),'Plan drift');plan=read(PLAN/'plan.json');manifest=None if args.rollback else evidence(plan['manifest'])
 if not(args.execute or args.rollback):
  before=sha(LIB/INDEX);verify_package(plan,manifest)
  if args.gate:verify_gate(args.gate,args.gate_sha,plan)
  require(sha(LIB/INDEX)==before,'Index changed during dry run')
  print(json.dumps(dict(status='FINAL_GATE_DRY_RUN_PASS'if args.gate else'PACKAGE_DRY_RUN_PASS_GATES_PENDING',new_files=310,reused_files=976,baseline_files=1300,index_before=before,plan_sha256=PLAN_SHA,manifest_sha256=plan['manifest']['sha256'],gate_sha256=args.gate_sha,library_modified=False)));return
 require(args.receipt is not None and not args.receipt.exists(),'Fresh explicit receipt path required')
 with(PLAN/'transaction.lock').open('a')as lock:
  fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
  require(sha(PLAN/'plan.json')==PLAN_SHA,'Plan changed before lock')
  if args.rollback:
   require(args.installation is not None and args.installation_sha==sha(args.installation),'Exact installation receipt required')
   old=plan['installed_index_sha256'];new=plan['staged_index']['sha256'];require(old==plan['baseline_index']['sha256'],'Plan baseline bindings differ');rollback_owned(LIB/INDEX,(PLAN/'installed34-index.json').read_bytes(),new,read(args.installation),old);record=dict(status='ROLLED_BACK_INDEX_ONLY',restored_sha256=sha(LIB/INDEX),immutable_resources_retained=True)
  else:
   old,new=verify_package(plan,manifest);gate=verify_gate(args.gate,args.gate_sha,plan);created=[]
   intent_path=args.receipt.with_name(args.receipt.name+'.intent.json');intent=dict(status='TRANSACTION_PREPARED',index_sha256=new,baseline_index_sha256=old,plan_sha256=PLAN_SHA,gate_sha256=args.gate_sha,script_sha256=sha(Path(__file__)),receipt_path=str(args.receipt.resolve()))
   intent_data=(json.dumps(intent,indent=2)+'\n').encode();atomic_absent(intent_path,intent_data,hashlib.sha256(intent_data).hexdigest())
   try:
    for row in manifest['files']:
     if atomic_absent(checked(LIB,row['path']),checked(STAGE,row['path']).read_bytes(),row['sha256']):created.append(row['path'])
    verify_package(plan,manifest);verify_gate(args.gate,args.gate_sha,plan)
    for row in manifest['files']:require(sha(checked(LIB,row['path']))==row['sha256'],'Installed immutable file mismatch')
    switch_index(LIB/INDEX,(STAGE/INDEX).read_bytes(),old,new);require(sha(LIB/INDEX)==new,'Switched index mismatch')
   except Exception as error:
    record=dict(intent,status='INDEX_SWITCHED_PENDING_RECOVERY'if sha(LIB/INDEX)==new else'FAILED_BEFORE_SWITCH_OR_EXTERNAL_INDEX',error=str(error),observed_index_sha256=sha(LIB/INDEX),created=created)
    data=(json.dumps(record,indent=2)+'\n').encode();atomic_absent(args.receipt,data,hashlib.sha256(data).hexdigest());raise
   record=dict(status='INSTALLED_PENDING_NORMAL_HTTP_PROOF',index_sha256=new,baseline_index_sha256=old,plan_sha256=PLAN_SHA,gate_sha256=args.gate_sha,final_static_map_sha256=gate['final_static']['map_sha256'],created=created,files_verified=310,reused_verified=976,baseline_nonindex_verified=1299,entries_added=7,static_files_modified=False,intent_sha256=sha(intent_path))
  data=(json.dumps(record,indent=2)+'\n').encode();atomic_absent(args.receipt,data,hashlib.sha256(data).hexdigest());print(json.dumps(record))
if __name__=='__main__':main()
