"""Prove the exact authorized animation and named-asset warning delta."""
import argparse,collections,copy,hashlib,json
from pathlib import Path
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement/restart2';D=B/'compiler-rebase-diagnosis-v2'
def read(p):return json.loads(p.read_text())
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 ap=argparse.ArgumentParser();ap.add_argument('candidate',type=Path);ap.add_argument('output',type=Path);ap.add_argument('--canonical',type=Path,default=D/'canonical.json');args=ap.parse_args();assert not args.output.exists();before=read(args.canonical);raw=read(args.candidate);after=raw.get('candidate',raw);plan=read(B/'combined-trio-stage-v2/publication-plan.json');contract=read(B/'combined-trio-stage-v2/animation-contract.json');pins={}
 def issues(lib,identity):
  p=lib/f'3d-assets/croisement03/{identity}/asset.json';pins[str(p.resolve())]=sha(p);return [f'Draft gameplay asset {identity}: {issue}'for issue in read(p)['gameplay']['draft']['issues']]
 compiler_pins_verified=False
 if 'compilerPins'in before:
  assert before['status']=='PASS_PINNED_CANONICAL_COMPILE' and raw['status']=='PASS_SAME_COMPILER_FULL_GAMEPLAY_AND_SCENERY'
  assert before['compilerPins']==raw['compilerPins'] and before['nodeVersion']==raw['nodeVersion']
  for name,h in before['compilerPins'].items():assert sha(Path(name))==h
  for name,h in before['inputPins'].items():assert sha(Path(name))==h
  compiler_pins_verified=True
 removed=collections.Counter(w for identity in plan['retire_map_references']for w in issues(R/'level-editor/library',identity));added=collections.Counter(w for identity in ['croisement03-tree-12','croisement03-tree-13','croisement03-tree-14']for w in issues(B/'combined-trio-stage-v2/library',identity))
 old=collections.Counter(before['geometry']['warnings']);new=collections.Counter(after['geometry']['warnings']);assert old-new==removed and new-old==added,'Unexpected warning delta'
 assert not before['geometry'].get('animations');assert before['scenerySources']==[];assert after['scenerySources']==[{'assetId':'croisement03-native-arbre06','animationId':'native-arbre06'}];assert after['geometry']['animations']==[contract['sourceAnimation']]
 expected=copy.deepcopy(before['geometry']);expected['animations']=[contract['sourceAnimation']];expected['warnings']=after['geometry']['warnings'];assert expected==after['geometry'],'Gameplay difference beyond exact authorized animation and fully provenance-checked draft warnings'
 report={'status':'PASS_CANONICAL_CURRENT_COMPILER_AUTHORIZED_DELTA_ONLY'if compiler_pins_verified else'PASS_FROZEN_PROCESS_DELTA_ONLY_NOT_CURRENT_SOURCE_PROOF','compiler_pins_verified':compiler_pins_verified,'before':{'path':str(args.canonical),'sha256':sha(args.canonical)},'after':{'path':str(args.candidate),'sha256':sha(args.candidate)},'descriptor_pins':pins,'retired_assets':plan['retire_map_references'],'removed_warning_multiset':dict(removed),'added_warning_multiset':dict(added),'all_other_gameplay_fields_exact':True,'doors_and_passage_states_exact':True,'animation_exact_contract':True,'warning_order':'Complete arrays retained in snapshots; multiset provenance checked against exact retired/new descriptor draft issues.','canonical_draft_normalization':'Exact approved dedup for east-tree-rocks/southeast-stone-wall only; no file writes.'};args.output.write_text(json.dumps(report,indent=2)+'\n');print({'status':report['status'],'output':str(args.output),'sha256':sha(args.output)})
if __name__=='__main__':main()
