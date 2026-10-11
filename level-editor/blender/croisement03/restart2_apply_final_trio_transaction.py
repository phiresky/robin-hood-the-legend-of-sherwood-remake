"""Root-only opt-in publication; default preparation never acquires the live lock."""
import argparse,importlib.util,json,shutil,subprocess,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
HERE=Path(__file__).resolve().parent
BASE=ROOT/'level-editor/work/croisement03-refinement/restart2'
RUNS=BASE/'final-trio-publication-transaction-v1'
LIB=ROOT/'level-editor/library';ASSETS=LIB/'3d-assets'
CORE=ROOT/'level-editor/blender/york/restart44_static_transaction.py'
import hashlib

def digest(p):
 with Path(p).open('rb')as f:return hashlib.file_digest(f,'sha256').hexdigest()
assert digest(CORE)=='a71796d93dbfbc8fd07ebd6deebf8bc0177e7e599a85b9233c43147fd38a30e4','Durable transaction implementation drift'
spec=importlib.util.spec_from_file_location('c3_durable_publication',CORE);protocol=importlib.util.module_from_spec(spec);spec.loader.exec_module(protocol)
core=protocol.core;require,sha,dump=core.require,core.sha,core.durable_json
EXECUTOR_SHA=sha(Path(__file__))
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from asset_index import validate_asset_index
PLANNER=HERE/'restart2_prepare_final_trio_transaction.py'
VALIDATOR=HERE/'restart2_validate_final_trio.mjs'

def read(p):return json.loads(p.read_text())
def pin(p):return {'path':str(p),'sha256':sha(p)}
def verify_records(rows,key):
 for row in rows:require(sha(Path(row['target']))==row[key],'Target drift: '+row['target'])
def editor_proof(path,expected,plan):
 require(path is not None and expected is not None,'Pinned editor interaction handoff required')
 require(sha(path)==expected,'Editor handoff hash differs')
 proof=read(path);require(str(proof.get('status','')).startswith('PASS'),'Editor handoff does not pass')
 require(proof.get('staged_map_sha256',proof.get('stage_map_sha256'))==plan['map_sha256'],'Editor handoff map differs')
 require(proof.get('approved_models')==plan['approved_models'],'Editor handoff model scope differs')
 checks=proof.get('editor_checks',{})
 for key in ('map_load','group_selection','part_selection','palette_insert','save_reload'):require(checks.get(key)is True,'Missing editor interaction: '+key)
 require(bool(proof.get('evidence')),'Editor evidence pins absent')
 for name,value in proof['evidence'].items():core.verify_pin({'path':name,'sha256':value})
 return pin(path)
def verify_evidence(plan):
 require(sha(Path(__file__))==EXECUTOR_SHA,'Executor changed during execution')
 for item in plan['protected']+plan['evidence']+[plan['browser']]:core.verify_pin(item)
 for name,value in plan['compiler_source_pins'].items():core.verify_pin({'path':name,'sha256':value})
 handoff=read(Path(plan['browser']['path']))
 require(handoff['approved_models']==plan['approved_models']and handoff['stage_map_sha256']==plan['map_sha256'],'Combined proof scope differs')
 for name,value in handoff['evidence'].items():core.verify_pin({'path':name,'sha256':value})
 for row in plan['records']:
  target=Path(row['target']);require(target.is_relative_to(LIB)and not target.is_symlink(),'Invalid target path')
  require(target.parent.resolve()==target.parent,'Symlink target ancestry')
  if row['source']:require(sha(Path(row['source']))==row['new_sha256'],'Source payload drift')
  else:require(row['operation']=='remove_obsolete_derivative'and row['old_sha256']is not None and row['new_sha256']is None,'Unpinned deletion')
def clean_directories(created):
 for path in reversed(created):
  directory=Path(path)
  if directory.exists() and not any(directory.iterdir()):directory.rmdir();core.fsync_dir(directory.parent)
def prepare(folder,compiler_baseline=None,compiler_sha256=None):
 subprocess.run([sys.executable,str(PLANNER),'--output',str(folder)]+(['--compiler-baseline',str(compiler_baseline),'--compiler-sha256',compiler_sha256]if compiler_baseline and compiler_sha256 else[]),cwd=ROOT,check=True)
 plan=read(folder/'plan.json')
 require(sha(folder/'plan.json')==read(folder/'dry-run.json')['plan']['sha256'],'Prepared plan drift')
 return plan

def main():
 ap=argparse.ArgumentParser();ap.add_argument('--output',type=Path,required=True);mode=ap.add_mutually_exclusive_group();mode.add_argument('--apply',action='store_true');mode.add_argument('--rollback',action='store_true');ap.add_argument('--root-reviewed',action='store_true');ap.add_argument('--readers-quiescent',action='store_true');ap.add_argument('--editor-proof',type=Path);ap.add_argument('--editor-sha256');ap.add_argument('--compiler-baseline',type=Path);ap.add_argument('--compiler-sha256');args=ap.parse_args();folder=args.output.resolve()
 require(folder.parent==RUNS,'Run directory outside C3 transaction scope')
 require(not(args.apply or args.rollback)or(args.root_reviewed and args.readers_quiescent),'Explicit root review and quiescent readers required')
 require(not args.apply or(args.editor_proof and args.editor_sha256),'Apply requires complete pinned editor interactions')
 require(bool(args.compiler_baseline)==bool(args.compiler_sha256),'Baseline reuse needs path and hash')
 if not(args.apply or args.rollback):
  plan=prepare(folder,args.compiler_baseline,args.compiler_sha256)
  if args.editor_proof:
   evidence=editor_proof(args.editor_proof.resolve(),args.editor_sha256,plan);dump(folder/'editor-proof-check.json',{'status':'PASS','proof':evidence})
  print(json.dumps({'status':'PREPARED_NOT_APPLIED','folder':str(folder)}));return
 with core.locked():
  if args.rollback:
   auth=read(folder/'execution-authority.json');require(sha(folder/'plan.json')==auth['plan']['sha256'],'Rollback plan changed')
   plan=read(folder/'plan.json');journal=read(folder/'transaction/journal.json')
   require(journal['plan_sha256']==auth['plan']['sha256'],'Rollback journal mismatch')
   require([(r['target'],r['old_sha256'],r['new_sha256'])for r in journal['files']]==[(r['target'],r['old_sha256'],r['new_sha256'])for r in plan['records']],'Rollback scope mismatch')
   core.restore(journal['files'],folder/'transaction',journal);clean_directories(read(folder/'directory-journal.json')['created']);print('ROLLED_BACK');return
  # Reprepare under lock from the latest catalog. The planner rejects C3 target drift
  # while retaining unrelated current rows, their order and top-level metadata.
  plan=prepare(folder,args.compiler_baseline,args.compiler_sha256);proof=editor_proof(args.editor_proof.resolve(),args.editor_sha256,plan);verify_evidence(plan);verify_records(plan['records'],'old_sha256')
  plan_hash=sha(folder/'plan.json');authority={'plan':pin(folder/'plan.json'),'executor':pin(Path(__file__)),'protocol':pin(CORE),'editor':proof,'root_reviewed':True,'readers_quiescent':True}
  dump(folder/'execution-authority.json',authority)
  reserve=sum((Path(r['source']).stat().st_size if r['source']else 0)+(Path(r['target']).stat().st_size if Path(r['target']).exists()else 0)for r in plan['records'])
  require(shutil.disk_usage(RUNS).free>reserve+2**30,'Insufficient durable rollback reserve')
  created=[];dump(folder/'directory-journal.json',{'created':created,'status':'PREPARING'})
  try:
   for name in plan['create_directories']:
    path=Path(name);require(path.is_relative_to(LIB)and not path.exists()and path.parent.resolve()==path.parent,'Directory scope changed');path.mkdir();core.fsync_dir(path.parent);created.append(name);dump(folder/'directory-journal.json',{'created':created,'status':'CREATED'})
   def precommit():
    require(sha(folder/'plan.json')==plan_hash,'Plan changed before commit');verify_evidence(plan);verify_records(plan['records'],'old_sha256');editor_proof(args.editor_proof.resolve(),args.editor_sha256,plan)
   def postcheck():
    verify_records(plan['records'],'new_sha256')
    for item in plan['protected']:core.verify_pin(item)
    validate_asset_index(ASSETS,read(ASSETS/'index.json'))
    subprocess.run(['node',str(VALIDATOR),str(LIB/'scenes/croisement03.rhlos-map.json'),str(LIB),str(folder/'installed-strict-validation.json'),str(folder/'same-compiler-baseline.json')],cwd=ROOT,check=True)
   journal=protocol.commit(plan['records'],folder/'transaction',plan_hash,precommit,postcheck)
   dump(folder/'result.json',{'status':'INSTALLED_LIVE_EDITOR_PROOF_PENDING','plan':authority['plan'],'journal':pin(folder/'transaction/journal.json'),'editor':proof,'canonical_map_sha256':sha(LIB/'scenes/croisement03.rhlos-map.json')})
  except BaseException:
   # The durable protocol restores any replacements, including deleted derivatives.
   # If failure preceded replacement, only these newly created empty directories exist.
   clean_directories(created);raise
  print(json.dumps({'status':'INSTALLED_LIVE_EDITOR_PROOF_PENDING','folder':str(folder),'targets':len(plan['records'])}))
if __name__=='__main__':main()
