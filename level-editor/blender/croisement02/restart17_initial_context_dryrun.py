"""Prepare and verify an initial-context publication proposal; no installation mode exists."""
import argparse,copy,hashlib,json
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'level-editor/work/croisement02-refinement/restart2-state'
LIB=ROOT/'level-editor/library'
CANDIDATE=BASE/'restart16-initial-context-v1'
DEFAULT=BASE/'restart17-initial-context-publication-v1'
INDEX='mission-states/index.json'

def require(ok,message):
    if not ok:raise ValueError(message)
def sha(path):
    h=hashlib.sha256()
    with Path(path).open('rb')as stream:
        for block in iter(lambda:stream.read(1024*1024),b''):h.update(block)
    return h.hexdigest()
def read(path):return json.loads(Path(path).read_text())
def safe(root,relative):
    base=Path(root).resolve();rel=Path(relative)
    require(not rel.is_absolute() and rel.parts and '..'not in rel.parts,'Unsafe relative path')
    path=base/rel
    require(path.resolve().is_relative_to(base),'Escaping path')
    for component in [path,*path.parents]:
        if component==base:break
        require(not component.is_symlink(),'Symlink path rejected')
    return path

def validate_contract_delta(before,after,ids):
    require(len(ids)==len(set(ids)),'Duplicate context IDs')
    changed=copy.deepcopy(after)
    states=changed['native'].get('patch_states',[])
    require(sum(s['id']in ids for s in states)==len(ids),'Missing added context')
    require(not any(s['id']in ids for s in before['native'].get('patch_states',[])),'Context already existed')
    changed['native']['patch_states']=[s for s in states if s['id']not in ids]
    if 'patch_states'not in before['native']:del changed['native']['patch_states']
    require(changed==before,'Existing contract data or physical bindings changed')

def validate_index_delta(before,after,records):
    require(len(before['entries'])==len(after['entries'])==41,'Entry count changed')
    require(len(records)==8 and len({r['id']for r in records})==8,'Wrong replacement count')
    by_id={r['id']:r for r in records};expected=copy.deepcopy(before)
    for entry in expected['entries']:
        if entry['id']in by_id:
            row=by_id[entry['id']]
            require(entry['contract']['path']==row['baseline_path'] and entry['contract']['sha256']==row['baseline_sha256'],'Wrong baseline reference')
            entry['contract']={**entry['contract'],'path':row['destination'],'sha256':row['sha256']}
    require(after==expected,'Catalog changed outside the8contract references')
    require(len({e['id']for e in after['entries']})==41,'Duplicate catalog IDs')

def runtime_paths(accepted):
    paths=set(accepted)
    for directory in ['level-editor/app/src','level-editor/shared/src']:
        for path in (ROOT/directory).rglob('*'):
            if path.is_file() and path.suffix in {'.ts','.tsx','.js','.mjs','.json','.wgsl','.glsl','.css'}:paths.add(str(path.relative_to(ROOT)))
    return sorted(paths)

def validate_candidate_record(row,reviewed):
    require(row['id']==reviewed['id'] and row['mission']==reviewed['mission'],'Candidate identity drift')
    require(row['sha256']==reviewed['sha256'] and row['baseline_sha256']==reviewed['baseline_sha256'],'Reviewed candidate digest drift')
    require(row['baseline_path']==reviewed['path'] and row['added_context_ids']==reviewed['added_context_ids'],'Reviewed candidate scope drift')
    require(row['source']==str((CANDIDATE/'library'/reviewed['path']).relative_to(ROOT)),'Candidate source path changed')
    require(row['destination']==f"mission-states/croisement02/contracts/initial-context/{reviewed['sha256']}.json",'Immutable destination changed')

def pin(path):return {'path':str(path.relative_to(ROOT)),'sha256':sha(path)}
def write_new(path,value):
    with path.open('x')as f:json.dump(value,f,indent=2);f.write('\n')

def prepare(output):
    manifest=read(CANDIDATE/'manifest.json');index=read(LIB/INDEX)
    require(sha(LIB/INDEX)==manifest['catalog_sha256'],'Catalog drift')
    review=read(CANDIDATE/'root-review-v1.json')
    require(review['status']=='PASS_ROOT_SOURCE_CONTEXT_CPU_REVIEW_BROWSER_PENDING'and review['manifest_sha256']==sha(CANDIDATE/'manifest.json'),'Root CPU review differs')
    proposed=copy.deepcopy(index);records=[]
    for row in manifest['records']:
        source=CANDIDATE/'library'/row['path'];require(sha(source)==row['sha256'],'Candidate drift')
        require(sha(LIB/row['path'])==row['baseline_sha256'],'Baseline contract drift')
        validate_contract_delta(read(LIB/row['path']),read(source),row['added_context_ids'])
        destination=f"mission-states/croisement02/contracts/initial-context/{row['sha256']}.json"
        record={'id':row['id'],'mission':row['mission'],'source':str(source.relative_to(ROOT)),'destination':destination,'sha256':row['sha256'],'baseline_path':row['path'],'baseline_sha256':row['baseline_sha256'],'added_context_ids':row['added_context_ids']};records.append(record)
        entry=next(e for e in proposed['entries']if e['id']==row['id']);entry['contract']={**entry['contract'],'path':destination,'sha256':row['sha256']}
    validate_index_delta(index,proposed,records)
    accepted_path=BASE/'installed41-normal-http-final-v1/verification.json';accepted=read(accepted_path);live_pins={};drift=[]
    for path,expected in accepted['normal_http_resources'].items():
        actual=sha(safe(LIB,path));live_pins[path]=actual
        if actual!=expected:
            require(not path.startswith('mission-states/'),'Accepted state resource drift')
            drift.append({'path':path,'accepted_sha256':expected,'current_sha256':actual})
    runtime_path=BASE/'installed41-normal-http-chunk-1-v1/runtime-baseline.json';runtime=read(runtime_path);runtime_pins={p:sha(safe(ROOT,p))for p in runtime_paths(runtime['source_files'])};runtime_drift=[{'path':p,'accepted_sha256':runtime['source_files'].get(p),'current_sha256':current}for p,current in runtime_pins.items()if runtime['source_files'].get(p)!=current]
    for entry in index['entries']:require(sha(safe(LIB,entry['contract']['path']))==entry['contract']['sha256'],'Installed contract does not match catalog')
    for row in manifest['reused']:require(sha(safe(LIB,row['path']))==row['sha256'],'Reused context resource drift')
    output.mkdir()
    with (output/'baseline41-index.json').open('xb')as f:f.write((LIB/INDEX).read_bytes())
    write_new(output/'proposed41-index.json',proposed)
    proof_names=['manifest.json','verification.json','visual-evidence.json','self-review.json','root-review-v1.json']
    plan={'status':'PRIVATE_PROPOSAL_BROWSER_AND_ROOT_PUBLICATION_PENDING','recipe':pin(Path(__file__)),'baseline_index_sha256':manifest['catalog_sha256'],'proposed_index':pin(output/'proposed41-index.json'),'baseline_index':pin(output/'baseline41-index.json'),'entries':41,'unchanged_entries':33,'changed_references':8,'records':records,'candidate_evidence':[pin(CANDIDATE/n)for n in proof_names],'accepted_browser_receipt':pin(accepted_path),'current_library_pins':live_pins,'current_runtime_pins':runtime_pins,'accepted_nonstate_drift':drift,'accepted_runtime_drift':runtime_drift,'reused_context_resources':manifest['reused'],'scope':'Raw pre-script source-art context only. Tac19mission patch015 is actor-dependent after startup. Existing geometry, textures, controls and physical bindings retain their previous approval scope. No new user approval or publication authorization inferred.','required_before_publication':['Browser owner must verify all8candidate entries against current runtime and library pins.','Root must review browser proof and current static/runtime deltas, then explicitly authorize its own guarded transaction.','Use immutable new contract destinations and catalog-last atomic switch; retain all old contract files.','No publication is implemented by this dry-run tool.']}
    write_new(output/'plan.json',plan)
    config={'status':'BROWSER_OWNER_HANDOFF_NOT_EXECUTED','plan':pin(output/'plan.json'),'installed_harness':str((BASE/'installed41-normal-http-chunk-1-v1/run.mjs').relative_to(ROOT)),'baseline_catalog_sha256':manifest['catalog_sha256'],'candidate_catalog':pin(output/'proposed41-index.json'),'entry_ids':[r['id']for r in records],'overlay':{'/library/'+INDEX:str((output/'proposed41-index.json').relative_to(ROOT)),**{'/library/'+r['destination']:r['source']for r in records}},'fallback':'All other /library resources served by the ordinary production library route; no copied assets or shared runtime overlay.','checks':['All41entry IDs present;33entries exactly unchanged;8selected contracts match proposed hashes.','Each changed entry: exact contract, native selection, pre-script initial, existing applied endpoint, reset and Map-only cleanup.','Native initial context has129/129source-patch coverage; no added family controls.','Tac06 added patches002/003 remain ordered at elevation3/5; Tac19patch015 at20.','Tac19patch000 remains transparent; no invented visible initial source.','All successful library HTTP resources and runtime pins rehashed after proof; current deltas explicitly reviewed.'],'scope':plan['scope'],'runtime_pins_in_plan':True,'user_facing_scope':'Raw pre-script initial artwork. Tac19 patch015 may be disabled by actor-dependent mission startup; this preview does not execute that script.','browser_prerequisites':['Shared-runtime owner confirms edits settled.','Browser owner confirms required free-memory reserve before launch.'],'runtime_inventory_scope':'Union of accepted inventory and all current app/src and shared/src source files; this may include unused private modules and does not claim each file is imported.'}
    write_new(output/'browser-config.json',config)
    return plan

def verify(output):
    plan=read(output/'plan.json');require(sha(safe(ROOT,plan['recipe']['path']))==plan['recipe']['sha256'],'Recipe drift');require(sha(LIB/INDEX)==plan['baseline_index_sha256'],'Catalog drift')
    before=read(safe(ROOT,plan['baseline_index']['path']));after=read(safe(ROOT,plan['proposed_index']['path']))
    for name in ['baseline_index','proposed_index']:
        row=plan[name];require(sha(safe(ROOT,row['path']))==row['sha256'],'Index proposal evidence drift')
    validate_index_delta(before,after,plan['records'])
    for row in plan['candidate_evidence']:
        require(sha(safe(ROOT,row['path']))==row['sha256'],'Candidate evidence drift')
    reviewed=read(CANDIDATE/'manifest.json');by_id={r['id']:r for r in reviewed['records']}
    require(set(by_id)=={r['id']for r in plan['records']},'Reviewed candidate membership differs')
    require({r['path']for r in plan['candidate_evidence']}=={str((CANDIDATE/n).relative_to(ROOT))for n in ['manifest.json','verification.json','visual-evidence.json','self-review.json','root-review-v1.json']},'Required evidence missing')
    accepted=read(safe(ROOT,plan['accepted_browser_receipt']['path']));require(sha(safe(ROOT,plan['accepted_browser_receipt']['path']))==plan['accepted_browser_receipt']['sha256'],'Accepted receipt drift')
    require(set(plan['current_library_pins'])==set(accepted['normal_http_resources']),'Library pin inventory changed')
    baseline_runtime=read(BASE/'installed41-normal-http-chunk-1-v1/runtime-baseline.json')
    require(set(plan['current_runtime_pins'])==set(runtime_paths(baseline_runtime['source_files'])),'Runtime source inventory changed')
    for row in plan['records']:
        validate_candidate_record(row,by_id[row['id']])
        source=safe(ROOT,row['source']);require(sha(source)==row['sha256'],'Candidate bytes drift');require(sha(safe(LIB,row['baseline_path']))==row['baseline_sha256'],'Old immutable contract drift')
        destination=safe(LIB,row['destination'])
        if destination.exists():require(sha(destination)==row['sha256'],'Conflicting immutable destination')
        validate_contract_delta(read(safe(LIB,row['baseline_path'])),read(source),row['added_context_ids'])
    for path,digest in plan['current_library_pins'].items():require(sha(safe(LIB,path))==digest,'Live resource changed: '+path)
    for path,digest in plan['current_runtime_pins'].items():require(sha(safe(ROOT,path))==digest,'Runtime changed: '+path)
    require(sha(LIB/INDEX)==plan['baseline_index_sha256'],'Concurrent catalog change')
    return {'status':'PASS_PACKAGE_DRY_RUN_BROWSER_PENDING','plan_sha256':sha(output/'plan.json'),'catalog_entries':41,'changed_contract_references':8,'old_contract_files_preserved':True,'library_resources_verified':len(plan['current_library_pins']),'runtime_files_verified':len(plan['current_runtime_pins']),'accepted_nonstate_drift':plan['accepted_nonstate_drift'],'accepted_runtime_drift':plan['accepted_runtime_drift'],'publication_ready':False,'library_modified':False,'publication_gates':plan['required_before_publication']}

def main():
    ap=argparse.ArgumentParser(description=__doc__);ap.add_argument('--prepare',action='store_true');ap.add_argument('--output',default=DEFAULT.name);a=ap.parse_args();output=safe(BASE,a.output)
    if a.prepare:prepare(output)
    report=verify(output)
    if a.prepare:write_new(output/'dry-run.json',report)
    print(json.dumps({k:v for k,v in report.items()if k not in ['accepted_nonstate_drift','accepted_runtime_drift','publication_gates']},sort_keys=True))
if __name__=='__main__':main()
