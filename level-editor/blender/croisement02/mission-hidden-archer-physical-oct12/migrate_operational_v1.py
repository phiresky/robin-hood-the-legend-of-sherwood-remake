"""Create explicit archer operational copies while retaining approved evidence bytes."""
import hashlib,json,sys
from pathlib import Path
HERE=Path(__file__).resolve().parent;ROOT=HERE.parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from work_relocation import RelocationMap
BASE=ROOT/'level-editor/work/active/croisement02/mission-hidden-archer-physical-oct12-v1'
DEST=ROOT/'level-editor/work/storage-maintenance/migration-20261012/croisement02/archers'
GEN='generation-short-no-mask-with-lighting-openrouter-with-auxiliary'
STATES=[f'{p}-{s}' for p in ['03','04'] for s in ['initial','transition-0','transition-1','applied']]
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def write(p,v):p.parent.mkdir(parents=True,exist_ok=True);p.write_text(json.dumps(v,indent=2)+'\n')
def main():
 assert not (DEST/'operational.json').exists()
 old='level-editor/work/croisement02-refinement';new='level-editor/work/active/croisement02'
 mapping=dict(version=1,path_mappings=[dict(old_prefix=old,new_prefix=new),dict(old_prefix=str(ROOT/old),new_prefix=str(ROOT/new))],historical_files=[],operational_files=[])
 r=RelocationMap(mapping,ROOT);records={};models=[]
 candidates=[BASE/'approved03-04-textures-v1/generation-batch.json',BASE/'approved03-04-textures-v1/prepared-batch.json',BASE/'actual-canopy05-compositions-v1/report.json',BASE/'actual-canopy05-compositions-v1/source-route-correction-v1.json']
 candidates += [BASE/n for n in ['qualification.json','profile01-native-loop-composition-v1/report.json','profile02-native-loop-composition-v1/report.json']]
 for state in STATES:
  exp=BASE/'approved03-04-textures-v1/experiments-v2'/state/'experiment'
  candidates += [exp/n for n in ['views.json','approval.json','preparation.json','auxiliary-references.json',GEN+'/generation-review.json',GEN+'/generation.json'] if (exp/n).is_file()]
  models.append(dict(state=state,path=str(exp/'approved-model.blend'),sha256=sha(exp/'approved-model.blend')))
 for model in [BASE/'geometry-v9-short-support03-v1/model.blend',BASE/'geometry-v8-short-support04-v4/model.blend',BASE/'geometry-v2-rounded02-recovered/profile-02/model.blend',BASE/'approved03-04-textures-v1/baked-v1/03-initial/model.blend']:
  assert model.is_file(),model
  models.append(dict(state='retained',path=str(model),sha256=sha(model)))
 for source in candidates:
  historical=sha(source);oldpath=str(source).replace('/work/active/croisement02/','/work/croisement02-refinement/')
  target=DEST/'operational-copies'/source.relative_to(BASE)
  value=r.translate_json(json.loads(source.read_text()));write(target,value)
  mapping['historical_files'].append(dict(old_path=oldpath,sha256=historical,preserved_path=str(source)))
  mapping['operational_files'].append(dict(old_path=oldpath,old_sha256=historical,new_path=str(target),new_sha256=sha(target)))
  records[str(source)]=dict(historical_sha256=historical,operational_path=str(target),operational_sha256=sha(target))
 write(DEST/'relocation.json',mapping)
 checks=RelocationMap(mapping,ROOT).verify()
 write(DEST/'operational.json',dict(version=1,records=records,models=models,verification=checks,mapping_sha256=sha(DEST/'relocation.json'),interrupted_review=dict(session=71879,exit_code=2,empty_directory=str(BASE/'approved03-04-textures-v1/baked-v1/03-initial/saved-review-v1')),generation_status='All eight paid generations complete; do not regenerate',resume_status='First saved appearance review pending; remaining seven bakes not started',canopy_handoff='Actual untrimmed sampler complete; static/moving diagnostic passes still required. All Archer01/02 occurrences precede Canopy05.'))
 print(json.dumps(checks))
if __name__=='__main__':main()
