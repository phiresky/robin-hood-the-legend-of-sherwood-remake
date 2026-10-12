"""Prepare a retained-profile reload with durable assertions before UI capture."""
from pathlib import Path
import hashlib,json,os,shutil
R=Path.cwd();B=R/'level-editor/work/croisement02-refinement/approved-round-wood-integration-oct11-v1';OLD=B/'editor-retained-reload-v1';O=B/'editor-retained-reload-v2';profile=B/'editor-check-v3/browser-v4/disposable-profile'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def dump(p,x):p.write_text(json.dumps(x,indent=2)+'\n')
O.mkdir(exist_ok=False)
for name in ['inputs.json','runtime-inputs.json','runtime-authority.json','source-snapshot.json','observe-library.js','expected-saved-document.json','reload-expectation.json']:
 shutil.copy2(OLD/name,O/name)
shutil.copytree(OLD/'snapshot',O/'snapshot',copy_function=os.link)
active=(profile/'DevToolsActivePort').read_text();port=int(active.splitlines()[0]);assert 1<=port<=65535
retained=json.loads((OLD/'retained-profile.json').read_text());retained.update(old_endpoint_port=port,devtools_active_port=active,lock_targets={n:os.readlink(profile/n)for n in ['SingletonLock','SingletonCookie','SingletonSocket']},terminal_owner='Root reports session41224 terminal143 after425s heartbeat, no SIGTERM event. Prior original OPFS proof is retained; actual profile liveness rechecked at launch.')
dump(O/'retained-profile.json',retained)
editor=(OLD/'editor.tsx').read_text().replace(str(OLD),str(O));(O/'editor.tsx').write_text(editor)
check=(OLD/'reload-check.js').read_text();check=check.replace('checks.push({name, pass: !!value, detail});','const row={name, pass: !!value, detail};checks.push(row);window.persistReloadEvidence(JSON.stringify({kind:"check",row}));')
needle="  window.scatterStage = {name: 'tree02-reloaded-insertion', time: Date.now()};"
check=check.replace(needle,"  const assertions={status:'RELOAD_ASSERTIONS_EXECUTED_CAPTURE_PENDING',checks,document:doc,capture:'tree02-reloaded-insertion'};window.reloadAssertionsResult=assertions;window.persistReloadEvidence(JSON.stringify({kind:'assertions-complete',result:assertions}));\n"+needle)
(O/'reload-check.js').write_text(check)
run=(OLD/'run.mjs').read_text().replace(str(OLD.relative_to(R)),str(O.relative_to(R))).replace('--window-size=1440,1000','--window-size=960,720').replace("'http://127.0.0.1:43377/json/version'","'http://127.0.0.1:'+retained.old_endpoint_port+'/json/version'")
run=run.replace("let endpointDead=false;", "if(await readFile(join(profile,'DevToolsActivePort'),'utf8')!==retained.devtools_active_port)throw Error('Retained endpoint changed');\nlet endpointDead=false;")
run=run.replace("await command('Runtime.addBinding',{name:'persistOpfsProof'});", "await command('Runtime.addBinding',{name:'persistReloadEvidence'});await command('Runtime.addBinding',{name:'persistOpfsProof'});")
run=run.replace("if(d.method==='Runtime.bindingCalled'&&d.params.name==='persistOpfsProof')", "if(d.method==='Runtime.bindingCalled'&&d.params.name==='persistReloadEvidence'){const payload=JSON.parse(d.params.payload);appendFileSync(join(out,'reload-assertions.jsonl'),JSON.stringify({...payload,receivedAt:new Date().toISOString()})+'\\n');if(payload.kind==='assertions-complete')appendFileSync(join(out,'assertions-complete.jsonl'),JSON.stringify(payload.result)+'\\n');}if(d.method==='Runtime.bindingCalled'&&d.params.name==='persistOpfsProof')")
run=run.replace("moduleResponses.push({...row,bytes:bytes.length,sha256:createHash('sha256').update(bytes).digest('hex')})", "const observed={...row,bytes:bytes.length,sha256:createHash('sha256').update(bytes).digest('hex')};moduleResponses.push(observed);persistObservation('runtime-module-responses.jsonl',observed)")
run=run.replace("await mkdir(out);", "await mkdir(out);await writeFile(join(out,'runtime-preflight.json'),JSON.stringify({runtime:await guardRuntime(),privatePins:Object.keys(privatePins).length},null,2));")
run=run.replace('stage:window.scatterStage,captures:window.scatterCaptures})','stage:window.scatterStage,captures:window.scatterCaptures,assertions:window.reloadAssertionsResult})')
run=run.replace("const png=await command('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});", "if(state.assertions?.status!=='RELOAD_ASSERTIONS_EXECUTED_CAPTURE_PENDING'||state.assertions.checks.length!==11||!state.assertions.checks.every(c=>c.pass))throw Error('Incomplete durable assertions');await writeFile(join(out,'reload-assertions-result.json'),JSON.stringify(state.assertions,null,2));await writeFile(join(out,'screenshot-pending.json'),JSON.stringify({name,requestedAt:new Date().toISOString(),width:960,height:720,fromSurface:false},null,2));const png=await command('Page.captureScreenshot',{format:'png',captureBeyondViewport:false,fromSurface:false});")
run=run.replace("await writeFile(join(out,name+'.png'),Buffer.from(png.data,'base64'));", "await writeFile(join(out,name+'.png'),Buffer.from(png.data,'base64'));await writeFile(join(out,'screenshot-complete.json'),JSON.stringify({name,completedAt:new Date().toISOString()},null,2));")
(O/'run.mjs').write_text(run)
inputs=json.loads((O/'inputs.json').read_text())
for p in [O/'editor.tsx',O/'reload-check.js',O/'retained-profile.json',OLD/'browser-v1/original-opfs-preopen.json',OLD/'browser-v1/reload-progress.json',OLD/'browser-v1/host-lifetime.jsonl']:
 inputs[str(p)]=sha(p)
dump(O/'inputs.json',inputs)
dump(O/'request.json',{'status':'PREPARED_NOT_LAUNCHED','runner':str(O/'run.mjs'),'runner_sha256':sha(O/'run.mjs'),'profile':str(profile),'origin':retained['origin'],'previous_endpoint':port,'assertions':'Each actual check via synchronous host CDP binding JSONL; full11check result persisted before screenshot request. Screenshot remains required, missing image cannot pass.','framing':'960x720 actual UI reload capture only; prior appearance evidence unchanged. fromSurface:false captures current view, avoiding compositor surface capture path.','diagnosis':'Prior stage reached01:38:47.280Z, final heartbeat01:41:27.104Z; no PNG saved, no SIGTERM receipt. ~160s spent after screenshot stage, host RSS149MB. This localizes pending work to screenshot/ack path but does not prove cause or completion. Default surface capture plus full-map SwiftShader rendering is plausible cost; no OS termination cause inferred.','guards':'Same frozen v3 runtime/stage, exact original OPFS comparison, current actual endpoint+singleton probes, complete postflight. Originalprofile retained.'})
print(O)
