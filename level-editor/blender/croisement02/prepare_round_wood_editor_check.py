"""Prepare the normal editor harness without starting a browser."""
from pathlib import Path
import hashlib,json,shutil
ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'level-editor/work/croisement02-refinement/approved-round-wood-integration-oct11-v1'
STAGE=BASE/'publication/stage-v2'
OUT=BASE/'editor-check-v1'
TEMPLATE=ROOT/'level-editor/work/croisement01-refinement/restart2/tree02-browser-private-v2'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def dump(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
def main():
 OUT.mkdir(exist_ok=False)
 old=str(TEMPLATE.relative_to(ROOT));new=str(OUT.relative_to(ROOT))
 run=(TEMPLATE/'run.mjs').read_text().replace(old,new).replace('level-editor/work/croisement01-refinement/restart2/tree02-publication-plan-v1/browser-private-library-v2',str((STAGE/'library').relative_to(ROOT))).replace("==='croisement01'","==='croisement02'").replace('level-editor/work/croisement01-refinement/restart2/tree02-browser-private-v1/reload-check.js',new+'/reload-check.js')
 start=run.index(' const bytes=await readFile(authority.index.path)');end=run.index('\n}',start)
 run=run[:start]+''' const bytes=await readFile(authority.index.path),hash=createHash('sha256').update(bytes).digest('hex');
 if(hash!==authority.index.initial_sha256)throw Error('Frozen private catalog drift');
 return {sha256:hash,unrelated_index_change:false};'''+run[end:]
 run=run.replace('i<1200','i<3600')
 (OUT/'run.mjs').write_text(run)
 (OUT/'editor.tsx').write_text((TEMPLATE/'editor.tsx').read_text().replace('croisement01','croisement02'))
 (OUT/'reload-check.js').write_text((TEMPLATE.parent/'tree02-browser-private-v1/reload-check.js').read_text())
 # Observe exact bytes without parsing a duplicate full catalog in the browser.
 observer=(TEMPLATE/'observe-library.js').read_text().replace('croisement01','croisement02').replace('(tree-02|group-005)','(tree-32|tree-38)')
 start=observer.index('        if (index) {');end=observer.index('        window.observedPublicationResponses',start)
 observer=observer[:start]+observer[end:]
 (OUT/'observe-library.js').write_text(observer)
 dirs=['level-editor/app/src','level-editor/app/tests','level-editor/shared/src','level-editor/shared/test-fixtures']
 snapshots={}
 for directory in dirs:
  for p in sorted((ROOT/directory).rglob('*')):
   if p.is_file() and '.test.' not in p.name:
    rel=str(p.relative_to(ROOT));snapshots[rel]=sha(p);target=OUT/'snapshot'/rel;target.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(p,target)
 dump(OUT/'source-snapshot.json',snapshots)
 dump(OUT/'runtime-inputs.json',snapshots)
 dump(OUT/'runtime-authority.json',{'sourceDirectories':dirs,'membership':sorted(snapshots),'excludedModules':[],'index':{'path':str(STAGE/'library/3d-assets/index.json'),'initial_sha256':sha(STAGE/'library/3d-assets/index.json')}})
 plan=json.loads((STAGE/'plan.json').read_text());inputs={r['source']:r['new_sha256'] for r in plan['records']};inputs[str(STAGE/'plan.json')]=sha(STAGE/'plan.json')
 dump(OUT/'inputs.json',inputs)
 scene=json.loads((STAGE/'croisement02.rhlos-map.json').read_text());ids=['croisement02-tree-32','croisement02-tree-38'];refs={r['id']:r for r in scene['assetSources'] if r['id'] in ids}
 config={'asset_ids':ids,'pins':refs,'placements':{r['id']:r for r in scene['placements'] if r['id'] in ids},'expected_groups':sum(not r.get('ungrouped',False) for r in scene['placements']),'expected_parts':{aid:[p['node'] for p in json.loads((STAGE/f'library/3d-assets/croisement02/{aid}/asset.json').read_text())['parts']] for aid in ids}}
 dump(OUT/'check-config.json',config)
 shutil.copy2(Path(__file__).with_name('round_wood_editor_check.js'),OUT/'check.js')
 dump(OUT/'browser-request.json',{'status':'EXECUTABLE_PREPARED_NOT_RUN','command':['node',str(OUT/'run.mjs')],'plan_sha256':sha(STAGE/'plan.json'),'materialized_library':str(STAGE/'library'),'checks':'Full Croisement02, both groups and every part, native contact source/reverse, both palette inserts, save and page reload.','runtime_sources':len(snapshots),'no_scope_reduction':True})
 print(OUT)
if __name__=='__main__':main()
