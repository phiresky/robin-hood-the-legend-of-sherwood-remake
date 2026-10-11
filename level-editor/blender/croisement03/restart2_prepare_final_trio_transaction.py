"""Prepare root publication records without locking or writing the live library."""
import argparse,hashlib,json,sys,subprocess
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from asset_index import write_asset_index,validate_asset_index
BASE=ROOT/'level-editor/work/croisement03-refinement/restart2'
LIB=ROOT/'level-editor/library';ASSETS=LIB/'3d-assets'
OLD=BASE/'combined-trio-stage-v2'
CANDIDATE=BASE/'combined-trio-inferred-crown-candidate-v1'
STAGED=CANDIDATE/'browser-library-v1/library'
HANDOFF=BASE/'combined-trio-full-editor-browser-v5/handoff.json'
HANDOFF_SHA='a9ae926cc5543976868bf9d3bf5596e77982a0d579e21f3e2e21311b41183d7a'
RUNS=BASE/'final-trio-publication-transaction-v1'

def sha(p):
 p=Path(p)
 if not p.exists():return None
 with p.open('rb')as f:return hashlib.file_digest(f,'sha256').hexdigest()
def read(p):return json.loads(Path(p).read_text())
def write(p,d):
 with p.open('x')as f:json.dump(d,f,separators=(',',':'));f.write('\n')
def require(test,message):
 if not test:raise RuntimeError(message)
def pin(p):return {'path':str(p),'sha256':sha(p)}
def main():
 ap=argparse.ArgumentParser();ap.add_argument('--output',type=Path,required=True);ap.add_argument('--compiler-baseline',type=Path);ap.add_argument('--compiler-sha256');args=ap.parse_args();out=args.output.resolve();require(out.parent==RUNS,'Output outside owned runs');require(not out.exists(),'Output exists')
 require(sha(HANDOFF)==HANDOFF_SHA,'Browser handoff drift');handoff=read(HANDOFF)
 for p,d in handoff['evidence'].items():require(sha(p)==d,'Browser evidence drift: '+p)
 oldplan=read(OLD/'publication-plan.json');oldexpected={r['destination']:r['expected_destination_sha256']for r in oldplan['writes']}
 stage_map=STAGED/'scenes/croisement03.rhlos-map.json';doc=read(stage_map);require(sha(stage_map)==handoff['stage_map_sha256'],'Unreviewed staged map')
 compiler=CANDIDATE/'final-compile-validation.json';validation=read(compiler);require(validation['mapSha256']==sha(stage_map)and validation['status'].startswith('PASS'),'Final strict compiler missing')
 RUNS.mkdir(exist_ok=True);out.mkdir()
 compiler_baseline=out/'same-compiler-baseline.json'
 require(not args.compiler_baseline or(args.compiler_sha256 and sha(args.compiler_baseline)==args.compiler_sha256),'Reuse baseline requires exact hash')
 subprocess.run(['node',str(ROOT/'level-editor/blender/croisement03/restart2_prepare_trio_compiler_baseline.mjs'),str(compiler_baseline)]+([str(args.compiler_baseline.resolve())]if args.compiler_baseline else[]),cwd=ROOT,check=True)
 compiler_guard=read(compiler_baseline);require(compiler_guard['status']=='PASS_SAME_COMPILER_FULL_GAMEPLAY_AND_SCENERY','Current compiler comparison missing')
 refs=doc['assetSources']+doc['sceneAssets'];modelrefs={r['id']:r for r in refs}
 for identity,digest in handoff['approved_models'].items():require(modelrefs[identity]['model_sha256']==digest and sha(STAGED/modelrefs[identity]['model'])==digest,'Model outside browser proof')
 protected=[];records=[];files={};changed_descriptors=[];affected=set()
 def add(source,target,digest):
  old=sha(target);require(sha(source)==digest,'Source drift: '+str(source));require(not target.is_symlink(),'Symlink target')
  if old==digest:protected.append({'path':str(target),'sha256':old});return
  require(str(target)in oldexpected and old==oldexpected[str(target)],'Canonical C3 destination changed or scope grew: '+str(target))
  records.append({'source':str(source),'target':str(target),'old_sha256':old,'new_sha256':digest,'operation':'replace'})
  if target.is_relative_to(ASSETS):files[target.relative_to(ASSETS).as_posix()]=source
 add(stage_map,LIB/'scenes/croisement03.rhlos-map.json',sha(stage_map))
 for ref in refs:
  for field in ('descriptor','model'):
   source=STAGED/ref[field];target=LIB/ref[field];digest=ref[field+'_sha256'];before=len(records);add(source,target,digest)
   if len(records)>before:
    affected.add(ref['id'])
    if field=='descriptor':changed_descriptors.append(Path(ref[field]).relative_to('3d-assets').as_posix())
  for resource in ref.get('resources',[]):add(STAGED/resource['path'],LIB/resource['path'],resource['sha256'])
 require(affected=={'croisement03-tree-12','croisement03-tree-13','croisement03-tree-14','croisement03-terrain','croisement03-native-arbre06','croisement03-east-tree-rocks','croisement03-southeast-stone-wall'},'Unexpected affected asset scope')
 index_pin=pin(ASSETS/'index.json');current=read(ASSETS/'index.json');retired=set(oldplan['retire_map_references'])
 # Only scoped assets use the final browser-proven original GLBs. Unrelated derivatives remain untouched.
 removals=set()
 for row in current['assets']:
  if row['id']not in affected:continue
  for key in ('lossy_model','preview_model'):
   if row.get(key):removals.update([row[key],row[key]+'.receipt.json'])
 for identity in affected:
  parent=Path(modelrefs[identity]['model']).relative_to('3d-assets').parent
  for stem in ('lossy.glb','preview.glb','model.lossy.glb','model.preview.glb'):
   removals.update([(parent/stem).as_posix(),(parent/(stem+'.receipt.json')).as_posix()])
 for relative in sorted(removals):
  files[relative]=None;target=ASSETS/relative
  if target.exists():require(not target.is_symlink(),'Symlink derivative');records.append({'source':None,'target':str(target),'old_sha256':sha(target),'new_sha256':None,'operation':'remove_obsolete_derivative'})
 write_asset_index(ASSETS,target=out/'affected-index.json',files=files,descriptors=sorted(changed_descriptors))
 replacement={r['id']:r for r in read(out/'affected-index.json')['assets']};require(set(replacement)==affected,'Affected index mismatch')
 merged={**current,'assets':[replacement.get(r['id'],r)for r in current['assets']if r['id']not in retired]}
 previous={r['id']for r in current['assets']};merged['assets'] += [replacement[k]for k in sorted(affected-previous)]
 require(all('lossy_model'not in r and 'preview_model'not in r for r in replacement.values()),'Unreviewed derivative selected')
 validate_asset_index(ASSETS,merged,files=files);write(out/'index.json',merged)
 require(sha(ASSETS/'index.json')==index_pin['sha256'],'Catalog changed during preparation; rerun to rebase')
 # Publish data first, map and compact rebased catalog last. Existing files are never edited through hardlinks.
 records.sort(key=lambda r:(Path(r['target']).name=='croisement03.rhlos-map.json',r['target']))
 records.append({'source':str(out/'index.json'),'target':str(ASSETS/'index.json'),'old_sha256':index_pin['sha256'],'new_sha256':sha(out/'index.json'),'operation':'replace'})
 for row in records:
  require(sha(row['target'])==row['old_sha256'],'Destination moved during prepare')
  if row['source']:require(sha(row['source'])==row['new_sha256'],'Source moved during prepare')
 for item in protected:require(sha(item['path'])==item['sha256'],'Protected input moved')
 unchanged=sum(r['id']not in affected|retired for r in current['assets'])
 missing_dirs=set()
 for row in records:
  parent=Path(row['target']).parent
  while parent!=LIB and not parent.exists():
   require(parent.is_relative_to(LIB),'Directory outside library');missing_dirs.add(str(parent));parent=parent.parent
  require(parent.resolve()==parent,'Symlink target ancestor')
 plan={'status':'PREPARED_PRIVATE_ROOT_TRANSACTION_INTERACTION_PROOF_PENDING','records':records,'protected':protected,'browser':pin(HANDOFF),'evidence':[pin(Path(__file__)),pin(ROOT/'level-editor/refinement/asset_index.py'),pin(ROOT/'level-editor/blender/croisement03/restart2_validate_final_trio.mjs'),pin(compiler),pin(compiler_baseline),pin(ROOT/'level-editor/blender/croisement03/restart2_prepare_trio_compiler_baseline.mjs'),pin(CANDIDATE/'receipt.json'),pin(CANDIDATE/'native-overlap-conservative-v3.json')],
 'approved_models':handoff['approved_models'],'map_sha256':sha(stage_map),'affected_assets':sorted(affected),'retired_index_rows':sorted(retired),'unrelated_catalog_rows_preserved':unchanged,
 'create_directories':sorted(missing_dirs,key=lambda p:(len(Path(p).parts),p)),'directory_policy':'Root creates only listed missing directories under lock, journaling creation; rollback removes only empty transaction-created directories. Existing parent ancestry must remain nonsymlink.','source_only':True,'index_policy':'Rebuild7affected rows from authoritative final descriptors; preserve current unrelated rows/order/top-level metadata. Remove7retired rows; append newlyintroduced rows. Reprepare if catalog hash drifts before apply.',
 'supersedes':pin(OLD/'publication-plan.json'),'canonical_written':False,
 'compiler_source_pins':compiler_guard['compilerPins'],
 'required_before_apply':['Root review of this exact plan and evidence','Fresh full-editor group/part selection, palette insertion and save/reload proof on exact staged map/models; current handoff does not cover these','Under shared publication lock rehash all old destinations, source/evidence/protected pins; require quiescent readers','Durable byte backups and journal for every existing target; record absent targets; refuse symlink parents; samefilesystem atomic replacements; rollback exact backups on failure','Install data before map/index; after commit rerun final strict validator and fresh canonical editor proof'],
 'remaining_interaction_scope':{'groups':['croisement03-tree-12','croisement03-tree-13','croisement03-tree-14','croisement03-native-arbre06'],'parts':'Select every child, including both static-native-samples and inferred-crown-context; gameplay-only wood IDs remain independently selectable.','palette':'Load exact-source previews and insert each of three trees plus native animation once at distinct poses. Confirm one native effect per animation placement; no frame0 proxies.','save_reload':'Save disposable inserted map, fresh-editor reload, preserve alloriginal placements/refs/resources and inserted poses, verify actual model/catalog/frame response hashes. Remove temporary instances only inside disposable fixture.','not_proven':'Automatic mission-driven source clock wiring remains outside this fixture-owned clock proof.'}}
 write(out/'plan.json',plan);write(out/'dry-run.json',{'status':'PASS_PREPARATION_ONLY','plan':pin(out/'plan.json'),'targets':len(records),'removals':sum(r['source']is None for r in records),'changed_assets':sorted(affected),'unrelated_index_rows':unchanged,'browser_clock_appearance_proven':True,'editor_interactions_proven':False})
 print(json.dumps(read(out/'dry-run.json')))
if __name__=='__main__':main()
