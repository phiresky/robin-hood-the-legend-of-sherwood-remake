"""Restore the recorded saved JSON for a bounded normal-editor reload check."""
import hashlib,json,os,shutil
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'level-editor/work/croisement02-refinement/approved-round-wood-integration-oct11-v1'
OLD=BASE/'editor-check-v2'
OUT=BASE/'editor-reload-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def dump(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
def main():
 OUT.mkdir(exist_ok=False)
 before=json.loads((OLD/'browser-v1/pre-reload-checkpoint.json').read_text())
 reloaded=json.loads((OLD/'browser-v1/reloaded-document.json').read_text())
 expected=before['evidence']['reloadExpectation']
 assert reloaded['groups']==expected['groups'] and reloaded['objects']==expected['objects']
 for aid,x in [('croisement02-tree-32-instance1',2400),('croisement02-tree-38-instance1',2800)]:
  assert next(g for g in reloaded['groups'] if g['id']==aid)['transform']['dx']==x
 dump(OUT/'restored-saved-document.json',before['evidence']['savedDocument'])
 dump(OUT/'reload-expectation.json',expected)
 for name in ['inputs.json','runtime-inputs.json','runtime-authority.json','source-snapshot.json','observe-library.js']:
  shutil.copy2(OLD/name,OUT/name)
 shutil.copytree(OLD/'snapshot',OUT/'snapshot',copy_function=os.link)
 old=str(OLD.relative_to(ROOT));new=str(OUT.relative_to(ROOT))
 run=(OLD/'run.mjs').read_text().replace(old,new)
 a=run.index(" await evaluate('window.tree02Config=");b=run.index(' await Promise.all(modulePending);',a)
 run=run[:a]+''' const prior=JSON.parse(await readFile('PRIOR_PATH','utf8'));
 await evaluate('window.tree02ReloadExpectation='+await readFile('NEW_PATH/reload-expectation.json','utf8')+';window.scatterCaptures={};window.scatterDone=false');
 const reloadScript=await readFile('NEW_PATH/reload-check.js','utf8');
 await evaluate('void ('+reloadScript+').then(result=>{window.reloadResult=result;window.scatterDone=true},error=>{window.scatterError=String(error);window.scatterDone=true})');
 for(let i=0;i<3600;i++){await new Promise(r=>setTimeout(r,500));const state=await evaluate('({done:window.scatterDone,error:window.scatterError,stage:window.scatterStage,captures:window.scatterCaptures})');if(state.error)throw Error(state.error);const name=state.stage?.name;if(name&&!state.captures[name]){const png=await command('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});await writeFile(join(out,name+'.png'),Buffer.from(png.data,'base64'));await evaluate('window.scatterCaptures['+JSON.stringify(name)+']=true');}if(state.done)break;}
 const reloadResult=await evaluate('window.reloadResult');if(reloadResult?.status!=='RELOAD_EXECUTED_PENDING_VISUAL_REVIEW')throw Error('Reload incomplete');
 await writeFile(join(out,'reload-result.json'),JSON.stringify(reloadResult,null,2));
 const result={status:'SCATTER_EXECUTED_PENDING_VISUAL_REVIEW',checks:[...prior.checks,...reloadResult.checks],evidence:{prior:prior.evidence,reload:reloadResult},scope:'Fresh normal-editor load of the exact saved JSON value restored from pinned prior evidence. Original browser profile was removed by previous fixture cleanup; this does not claim profile reuse.'};
'''.replace('PRIOR_PATH',old+'/browser-v1/pre-reload-checkpoint.json').replace('NEW_PATH',new)+run[b:]
 run=run.replace("if(chrome.exitCode===0)await rm(profile,{recursive:true,force:false});",'// Retain this continuation profile, including on fixture failure.\n')
 (OUT/'run.mjs').write_text(run)
 editor=(OLD/'editor.tsx').read_text()
 needle="const library = await openHttpLibrary('/library/');"
 restore="""const recovered = await (await fetch('/@fsRESTORE_PATH')).text();
const storage = await navigator.storage.getDirectory();
const workspace = await storage.getDirectoryHandle('sherwood-level-editor', {create:true});
const savedMaps = await workspace.getDirectoryHandle('maps', {create:true});
const savedHandle = await savedMaps.getFileHandle('croisement02.rhlos-map.json', {create:true});
const savedWriter = await savedHandle.createWritable(); await savedWriter.write(recovered); await savedWriter.close();
sessionStorage.setItem('tree01-reopen','true');
""".replace('RESTORE_PATH',str(OUT/'restored-saved-document.json'))
 assert needle in editor
 (OUT/'editor.tsx').write_text(editor.replace(needle,restore+needle))
 check=(OLD/'reload-check.js').read_text()
 a=check.index('  const row = [...document.querySelectorAll');b=check.index("  window.scatterStage =",a)
 check=check[:a]+'''  const filter = document.querySelector('[aria-label="Find scene objects"]');
  filter.value = ''; filter.dispatchEvent(new Event('input', {bubbles:true}));
  await until(() => document.querySelectorAll('.object-list li.depth-0').length >= doc.groups.length, 'unfiltered group list');
  const rows = [...document.querySelectorAll('.object-list li.depth-0')];
  for (const [id, x] of [['croisement02-tree-32-instance1',2400], ['croisement02-tree-38-instance1',2800]]) {
    const index = doc.groups.findIndex(g => g.id === id), group = doc.groups[index];
    check('Reload preserves inserted X '+id, group?.transform.dx === x, group?.transform);
    const row = rows[index];
    check('Document group order matches actual UI row '+id, index >= 0 && row?.title === (group.name ?? group.id) && !!row.querySelector('.chev-btn'));
    row.scrollIntoView({block:'center'}); row.click();
    await until(() => v.bindings.selection()?.kind === 'group' && v.bindings.selection()?.id === id, 'exact inserted group UI selection '+id);
    check('Actual selected group identity '+id, v.bindings.selection()?.id === id);
  }
'''+check[b:]
 (OUT/'reload-check.js').write_text(check)
 inputs=json.loads((OUT/'inputs.json').read_text())
 for p in [OLD/'browser-v1/pre-reload-checkpoint.json',OLD/'browser-v1/reloaded-document.json',OUT/'restored-saved-document.json',OUT/'reload-expectation.json',OUT/'reload-check.js']:
  inputs[str(p)]=sha(p)
 dump(OUT/'inputs.json',inputs)
 dump(OUT/'request.json',{'status':'CPU_READY_NOT_RUN','command':['node',str(OUT/'run.mjs')],'prior_checks':len(before['checks']),'prior_captures':len(before['evidence']['captures']),'preserved_prior_reload_groups_parts':True,'both_inserted_x_verified_in_prior_reload':True,'restoration':'Saved JSON value extracted from pre-reload-checkpoint, serialized to a pinned file. Original OPFS bytes/profile are unavailable; no claim of original byte formatting or profile reuse.','run_sha256':sha(OUT/'run.mjs'),'checks_sha256':sha(OUT/'reload-check.js'),'prior_evidence':{str(p):sha(p) for p in [OLD/'browser-v1/pre-reload-checkpoint.json',OLD/'browser-v1/reloaded-document.json']}})
 print(OUT)
if __name__=='__main__':main()
