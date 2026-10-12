"""Add independent installed-state checks to the fresh private tree-map proof."""
from pathlib import Path
import json,hashlib
R=Path.cwd();B=R/'level-editor/work/active/croisement02';O=B/'approved-round-wood-integration-oct11-v1/editor-check-v3';P=B/'restart136-installed-state-ui-inputs-v1/inputs.json'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def dump(p,x):p.write_text(json.dumps(x,indent=2)+'\n')
p=json.loads(P.read_text());inputs=json.loads((O/'inputs.json').read_text())
for row in p['pins']:
 f=Path(row['path']);assert sha(f)==row['sha256'];inputs[str(f)]=row['sha256']
 if '/library/' in str(f):
  staged=B/'approved-round-wood-integration-oct11-v1/publication/stage-v3-current/library'/f.relative_to(R/'level-editor/library');assert sha(staged)==row['sha256'];inputs[str(staged)]=row['sha256']
inputs[str(P)]=sha(P);dump(O/'inputs.json',inputs)
run=(O/'run.mjs').read_text();marker=" await evaluate('window.tree02Config=";assert run.count(marker)==1
prefix="""
 const statePacket=JSON.parse(await readFile(STATE_PACKET,'utf8'));
 const stateScope=statePacket.scope;
 await evaluate('window.stateInitialCamera=(()=>{const v=window.reviewViewport;return {position:v.camera.position.clone(),quaternion:v.camera.quaternion.clone(),target:v.orbit.target.clone(),frustum:v.frustum,zoom:v.camera.zoom}})()');
 for(const name of statePacket.order){
  const cfgPath=statePacket.pins.find(x=>x.path.endsWith('/'+name+'-config.json')).path;
  const jsPath=statePacket.pins.find(x=>x.path.endsWith('/'+name+'-check.js')).path;
  await evaluate('window.stateUiPacket='+await readFile(cfgPath,'utf8')+';window.scatterCaptures={};window.scatterStage=null;window.stateCheckDone=false;window.stateCheckError=null');
  const stateScript=await readFile(jsPath,'utf8');
  await evaluate('void ('+stateScript+').then(result=>{window.stateCheckResult=result;window.stateCheckDone=true},error=>{window.stateCheckError=String(error);window.stateCheckDone=true})');
  let finished=false;
  for(let i=0;i<3600;i++){
   await new Promise(r=>setTimeout(r,500));
   const progress=await evaluate('({done:window.stateCheckDone,error:window.stateCheckError,stage:window.scatterStage,captures:window.scatterCaptures})');
   await writeFile(join(out,name+'-progress.json'),JSON.stringify(progress,null,2));
   if(progress.error){await writeFile(join(out,name+'-failure-evidence.json'),JSON.stringify(await evaluate('window.stateUiEvidence'),null,2));throw Error(progress.error)}
   const capture=progress.stage?.name;
   if(capture&&!progress.captures[capture]){if(!statePacket.capture_names.includes(capture))throw Error('Unexpected state capture '+capture);const shot=await command('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});await writeFile(join(out,capture+'.png'),Buffer.from(shot.data,'base64'));await evaluate('window.scatterCaptures['+JSON.stringify(capture)+']=true')}
   if(progress.done){finished=true;break}
  }
  if(!finished)throw Error('State check timeout '+name);
  const evidence=await evaluate('window.stateCheckResult');
  await writeFile(join(out,name+'-executed.json'),JSON.stringify({...evidence,combinedScope:stateScope},null,2));
  if(evidence?.status!=='SCATTER_EXECUTED_PENDING_VISUAL_REVIEW'||!evidence.checks.every(x=>x.pass))throw Error('State check incomplete '+name);
  await observationWrites;
 }
 await evaluate(`(()=>{const v=window.reviewViewport;if(v.bindings.document()?.mission?.importedFrom||v.missionStates.players.size)throw Error('State checks did not retire');v.select(null);if(window.stateInitialCamera)v.applyState(window.stateInitialCamera);const b=[...document.querySelectorAll('.editor-modes button')].find(b=>b.textContent.trim()==='Assets');if(!b)throw Error('Missing Assets mode');b.click();window.scatterStage=null;})()`);
""".replace('STATE_PACKET',json.dumps(str(P)))
run=run.replace(marker,prefix+marker);(O/'run.mjs').write_text(run)
o=(O/'observe-library.js').read_text().replace('if (response.ok && (index || selected))',"if (response.ok && (index || selected || path.startsWith('/library/mission-states/')))");(O/'observe-library.js').write_text(o)
r=json.loads((O/'browser-request.json').read_text());r.update(status='COMBINED_SCOPED_EXECUTABLE_READY_NOT_RUN',installed_state_inputs={'path':str(P),'sha256':sha(P)},state_scope=p['scope'],independent_results=['climbing-executed.json','tree42-executed.json'],required_followup='Short canonical-map smoke after root publication; this private-map proof is not canonical-map verification.')
r['runner_pins']={str(O/f):sha(O/f) for f in ['run.mjs','check.js','reload-check.js','observe-library.js','inputs.json','runtime-inputs.json','source-snapshot.json','check-config.json']};dump(O/'browser-request.json',r)
print(sha(O/'browser-request.json'))
