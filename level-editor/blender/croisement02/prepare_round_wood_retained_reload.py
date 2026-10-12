"""Prepare strict-origin continuation of the retained original OPFS profile."""
from pathlib import Path
import json,hashlib,os,shutil
R=Path.cwd();B=R/'level-editor/work/croisement02-refinement/approved-round-wood-integration-oct11-v1';OLD=B/'editor-check-v3';PRIOR=OLD/'browser-v4';O=B/'editor-retained-reload-v1'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def dump(p,x):p.write_text(json.dumps(x,indent=2)+'\n')
O.mkdir(exist_ok=False);before=json.loads((PRIOR/'pre-reload-checkpoint.json').read_text());expected=before['evidence']['reloadExpectation'];reloaded=json.loads((PRIOR/'reloaded-document.json').read_text());assert reloaded['groups']==expected['groups']and reloaded['objects']==expected['objects']
for aid,pin in expected['sourcePins'].items():assert next(r for r in reloaded['assetSources']if r['id']==aid)['model_sha256']==pin['model_sha256']
dump(O/'expected-saved-document.json',before['evidence']['savedDocument']);dump(O/'reload-expectation.json',expected)
for f in ['inputs.json','runtime-inputs.json','runtime-authority.json','source-snapshot.json','observe-library.js','reload-check.js']:shutil.copy2(OLD/f,O/f)
shutil.copytree(OLD/'snapshot',O/'snapshot',copy_function=os.link)
profile=PRIOR/'disposable-profile';assert profile.is_dir();port=(profile/'DevToolsActivePort').read_text();assert port.startswith('43377\n');locks={n:os.readlink(profile/n)for n in ['SingletonLock','SingletonCookie','SingletonSocket']};dump(O/'retained-profile.json',{'profile':str(profile),'origin':'http://127.0.0.1:35987','old_endpoint_port':43377,'lock_targets':locks,'terminal_owner':'Root reports unified session52915 terminal exit143, no host SIGTERM record; endpoint43377 unavailable. This is reported provenance, not fabricated local process evidence.'})
old=str(OLD.relative_to(R));new=str(O.relative_to(R));run=(OLD/'run-v4.mjs').read_text().replace(old,new).replace(new+'/browser-v4',new+'/browser-v1')
run=run.replace("import {readFile,writeFile,appendFile,mkdir,rm,statfs,readdir}","import {readFile,writeFile,appendFile,mkdir,rm,statfs,readdir,readlink,unlink}")
run="import {createConnection} from 'node:net';\n"+run
run=run.replace("port:0,watch:null", "port:35987,strictPort:true,watch:null")
run=run.replace("profile=join(out,'disposable-profile');await mkdir(profile);", "profile="+json.dumps(str(profile))+";\n"+'''const retained=JSON.parse(await readFile(RETAINED,'utf8'));
if(origin!==retained.origin)throw Error('Original origin not retained');
let endpointDead=false;try{await fetch('http://127.0.0.1:43377/json/version',{signal:AbortSignal.timeout(2000)})}catch{endpointDead=true}
if(!endpointDead)throw Error('Original Chrome endpoint still responds; profile reuse refused');
const socketDead=await new Promise((resolve,reject)=>{const s=createConnection(retained.lock_targets.SingletonSocket);s.once('connect',()=>{s.destroy();resolve(false)});s.once('error',e=>{if(['ENOENT','ECONNREFUSED'].includes(e.code))resolve(true);else reject(e)});s.setTimeout(2000,()=>{s.destroy();reject(Error('Original socket probe inconclusive'))})});
if(!socketDead)throw Error('Original profile socket still responds');
for(const[name,target]of Object.entries(retained.lock_targets))if(await readlink(join(profile,name))!==target)throw Error('Retained profile lock changed '+name);
await writeFile(join(out,'stale-lock-evidence.json'),JSON.stringify({endpointDead,socketDead,locks:retained.lock_targets,priorTerminalOwner:retained.terminal_owner},null,2));
for(const name of Object.keys(retained.lock_targets))await unlink(join(profile,name));
'''.replace('RETAINED',json.dumps(str(O/'retained-profile.json'))))
a=run.index(' const statePacket=');z=run.index(' await Promise.all(modulePending);',a)
run=run[:a]+''' const prior=JSON.parse(await readFile(PRIOR,'utf8'));
 await writeFile(join(out,'original-opfs-preopen.json'),JSON.stringify(await evaluate('window.originalOpfsProof'),null,2));
 await evaluate('window.tree02ReloadExpectation='+await readFile(EXPECTED,'utf8')+';window.scatterCaptures={};window.scatterDone=false;window.scatterStage=null');
 const reloadScript=await readFile(CHECK,'utf8');
 await evaluate('void ('+reloadScript+').then(result=>{window.reloadResult=result;window.scatterDone=true},error=>{window.scatterError=String(error);window.scatterDone=true})');
 for(let i=0;i<2400;i++){await new Promise(r=>setTimeout(r,250));const state=await evaluate('({done:window.scatterDone,error:window.scatterError,stage:window.scatterStage,captures:window.scatterCaptures})');await writeFile(join(out,'reload-progress.json'),JSON.stringify(state,null,2));if(state.error)throw Error(state.error);const name=state.stage?.name;if(name&&!state.captures[name]){if(name!=='tree02-reloaded-insertion')throw Error('Unexpected reload capture');const png=await command('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});await writeFile(join(out,name+'.png'),Buffer.from(png.data,'base64'));await evaluate('window.scatterCaptures['+JSON.stringify(name)+']=true');}if(state.done)break;}
 const reloadResult=await evaluate('window.reloadResult');if(reloadResult?.status!=='RELOAD_EXECUTED_PENDING_VISUAL_REVIEW')throw Error('Reload incomplete');await writeFile(join(out,'reload-result.json'),JSON.stringify(reloadResult,null,2));
 const result={status:'SCATTER_EXECUTED_PENDING_VISUAL_REVIEW',checks:[...prior.checks,...reloadResult.checks],evidence:{prior:prior.evidence,reload:reloadResult},scope:'Original retained OPFS profile and exact original origin. Saved JSON checked before opening, then exact groups/parts/pins and both inserted poses and UI selection. Prior state checks remain separately pinned.'};
'''.replace('PRIOR',json.dumps(str(PRIOR/'pre-reload-checkpoint.json'))).replace('EXPECTED',json.dumps(str(O/'reload-expectation.json'))).replace('CHECK',json.dumps(str(O/'reload-check.js')))+run[z:]
run=run.replace('i<3600','i<2200').replace("if(chrome.exitCode===0)await rm(profile,{recursive:true,force:false});",'/* Retain original profile on all outcomes. */')
run=run.replace("await command('Runtime.addBinding',{name:'persistLibraryDigest'});","await command('Runtime.addBinding',{name:'persistOpfsProof'});await command('Runtime.addBinding',{name:'persistLibraryDigest'});")
run=run.replace("if(d.method==='Runtime.bindingCalled'&&d.params.name==='persistLibraryDigest')","if(d.method==='Runtime.bindingCalled'&&d.params.name==='persistOpfsProof')persistObservation('original-opfs-preopen.jsonl',JSON.parse(d.params.payload));if(d.method==='Runtime.bindingCalled'&&d.params.name==='persistLibraryDigest')")
(O/'run.mjs').write_text(run)
editor=(OLD/'editor.tsx').read_text();needle="const library = await openHttpLibrary('/library/');"
pre='''const storage=await navigator.storage.getDirectory();
const workspace=await storage.getDirectoryHandle('sherwood-level-editor');
const savedMaps=await workspace.getDirectoryHandle('maps');
const savedHandle=await savedMaps.getFileHandle('croisement02.rhlos-map.json');
const savedBytes=await(await savedHandle.getFile()).arrayBuffer();
const actual=JSON.parse(new TextDecoder().decode(savedBytes));
const expected=await(await fetch(EXPECTED_URL)).json();
const stable=(x:any):any=>Array.isArray(x)?x.map(stable):x&&typeof x==='object'?Object.fromEntries(Object.keys(x).sort().map(k=>[k,stable(x[k])])):x;
if(JSON.stringify(stable(actual))!==JSON.stringify(stable(expected)))throw Error('Original OPFS saved map differs from pinned successful save');
Object.assign(window,{originalOpfsProof:{exactSavedJsonValue:true,bytes:savedBytes.byteLength,sha256:[...new Uint8Array(await crypto.subtle.digest('SHA-256',savedBytes))].map(n=>n.toString(16).padStart(2,'0')).join(''),origin:location.origin,restorationPerformed:false}});
(window as any).persistOpfsProof(JSON.stringify((window as any).originalOpfsProof));
sessionStorage.setItem('tree01-reopen','true');
'''.replace('EXPECTED_URL',json.dumps('/@fs'+str(O/'expected-saved-document.json')))
assert needle in editor;(O/'editor.tsx').write_text(editor.replace(needle,pre+needle))
inputs=json.loads((O/'inputs.json').read_text())
for f in [PRIOR/'pre-reload-checkpoint.json',PRIOR/'reloaded-document.json',PRIOR/'climbing-executed.json',PRIOR/'tree42-executed.json',OLD/'run-v4.mjs',O/'expected-saved-document.json',O/'reload-expectation.json',O/'reload-check.js',O/'retained-profile.json',O/'editor.tsx']:inputs[str(f)]=sha(f)
dump(O/'inputs.json',inputs);dump(O/'request.json',{'status':'PREPARED_NOT_LAUNCHED','run':str(O/'run.mjs'),'run_sha256':sha(O/'run.mjs'),'prior_checks':len(before['checks']),'original_profile':str(profile),'origin':'http://127.0.0.1:35987','strict_port':True,'opfs':'Read-only exact saved JSON comparison before editor/library load; no restoration. Runtime groups/parts/sourcepins checked before actual inserted-row clicks.','stale_locks':'Only three exact old symlinks removed at launch, only after endpoint dead and Unixsocket ENOENT/ECONNREFUSED. Inconclusive/live probe fails closed.','source_snapshot':'Same frozen v3 runtime, exact unchanged stage-v3 library.','remaining':'Root/solebrowser launch, inspect reload screenshot, verify complete HTTP/runtime postflight. All previous outputs retained.'});print(O)
