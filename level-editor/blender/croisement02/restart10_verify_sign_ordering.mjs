/** Private five-instance proof using the application's unchanged state layer. */
import {spawn} from 'node:child_process';
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import {createHash} from 'node:crypto';
import {createServer} from '../../app/node_modules/vite/dist/node/index.js';
import {chromeEndpoint,socketOpen,evaluate} from '../../app/tests/cdp.mjs';

const repo=resolve('.'),root=resolve('level-editor/work/croisement02-refinement/restart10-physical-signs');
const input=join(root,'input-ordering-v1'),output=join(root,'ordering-browser-v1');
await mkdir(output);await mkdir(join(output,'profile'));
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
const protectedFiles=['level-editor/library/mission-states/index.json','level-editor/library/scenes/croisement02.rhlos-map.json',
 'level-editor/app/src/mission-state-layer.ts','level-editor/app/src/state-appearance-player.ts'];
const protectedPins=Object.fromEntries(await Promise.all(protectedFiles.map(async p=>[p,hash(await readFile(p))])));
const html=`<!doctype html><meta charset="utf-8"><title>Private physical sign proof</title><style>body{margin:0;background:#323232}</style><script type="module" src="/@fs/${repo}/level-editor/blender/croisement02/restart10_sign_ordering_viewer.ts"></script>`;
await writeFile(join(input,'index.html'),html);
const vite=await createServer({configFile:false,root:input,publicDir:false,cacheDir:join(output,'vite-cache'),
 resolve:{alias:{'three':join(repo,'level-editor/app/node_modules/three'),'@rle/shared':join(repo,'level-editor/shared/src/index.ts')}},
 server:{host:'127.0.0.1',port:0,fs:{allow:[repo]}},logLevel:'warn'});
await vite.listen();const port=vite.httpServer.address().port;
const chrome=spawn('/usr/lib/chromium/chromium',['--headless','--no-sandbox','--disable-dev-shm-usage','--disable-background-networking',
 '--enable-unsafe-swiftshader','--use-angle=swiftshader','--window-size=512,512','--remote-debugging-port=0',
 '--user-data-dir='+join(output,'profile'),`http://127.0.0.1:${port}/`],{stdio:['ignore','ignore','pipe']});
let logs='',ws,id=0;chrome.stderr.on('data',d=>logs+=d.toString());const closed=new Promise(r=>chrome.on('close',r));
async function call(code){return evaluate(ws,++id,code,{timeoutMs:60000})}
function checkStates(states,phase){
 if(states.length!==5)throw Error('Expected five independent signs');
 for(const row of states){
  const bodies=row.active.filter(n=>n.body!==null),shadows=row.active.filter(n=>n.shadow!==null);
  const expected=typeof phase==='object'?phase[row.id]:phase;
  if(bodies.length!==2||shadows.length!==1||bodies.some(n=>n.body!==expected)||shadows.some(n=>n.shadow!==expected))
   throw Error('Incorrect physical phase '+JSON.stringify({row,expected}));
 }
}
try{
 const endpoint=new URL(await chromeEndpoint(chrome));const pages=await(await fetch('http://'+endpoint.host+'/json/list')).json();
 ws=new WebSocket(pages.find(p=>p.type==='page').webSocketDebuggerUrl);await socketOpen(ws);
 let status;
 for(let i=0;i<360;i++){
  status=await call('window.physicalSigns ? {loaded:physicalSigns.loaded,stage:physicalSigns.stage,failedStage:physicalSigns.failedStage,error:physicalSigns.error} : null');
  if(status?.loaded||status?.error)break;
  if(i%20===0)console.log(JSON.stringify(status));
  await new Promise(r=>setTimeout(r,500));
 }
 if(!status?.loaded)throw Error('Loading failed '+JSON.stringify(status));
 const contract=await call('physicalSigns.contract');await writeFile(join(output,'physical-contract.json'),JSON.stringify(contract,null,2)+'\n');
 const nativeManifestPath=resolve('level-editor/work/croisement02-refinement/state-target-evidence/manifest.json');
 const nativeBytes=await readFile(nativeManifestPath),nativeManifest=JSON.parse(nativeBytes);
 const rows=nativeManifest.profiles.find(p=>p.id==='TG_Panel-12').rows;
 const fields=['image_sha256','offset','size','delay','ticks','sound_id'];
 const signatures=rows.map(row=>JSON.stringify(row.frames.map(f=>Object.fromEntries(fields.map(k=>[k,f[k]])))));
 if(rows.length!==3||rows.map(r=>r.action_id).join(',')!=='0,210,211'||new Set(signatures).size!==1)throw Error('Source action aliases differ');
 for(const row of rows)for(const f of row.frames)if(hash(await readFile(f.image))!==f.image_sha256)throw Error('Native sign frame changed');
 const aliases=[];
 for(const action of [0,210,211]){
  const values=await call(`(()=>{for(const [id]of physicalSigns.layer.players)physicalSigns.layer.selectAction(id,${action});physicalSigns.seek(16);return physicalSigns.states()})()`);
  checkStates(values,8);aliases.push({action,states:values});
 }
 await call("for(const [id]of physicalSigns.layer.players)physicalSigns.layer.selectAction(id,0)");
 const samples=[];
 for(let phase=0;phase<32;phase++){
  const states=await call(`(()=>{physicalSigns.seek(${phase*2});return physicalSigns.states()})()`);checkStates(states,phase);samples.push({phase,states});
 }
 const uuids=samples[0].states.flatMap(r=>r.nodeUUIDs);
 if(new Set(uuids).size!==480)throw Error('Cloned signs share animation node identities');
 const independent=await call("(()=>{physicalSigns.seek(0);physicalSigns.layer.seek('sign-7',16);return physicalSigns.states()})()");
 checkStates(independent,Object.fromEntries(independent.map(r=>[r.id,r.id==='sign-7'?8:0])));
 await call('physicalSigns.seek(63)');const wrapped=await call('physicalSigns.advance(.04)');checkStates(wrapped,0);
 const before=await call('physicalSigns.replacements()');
 await call('physicalSigns.clear()');const cleared=await call('({replaced:physicalSigns.replacements(),children:physicalSigns.layer.root.children.length})');
 if(cleared.replaced.length||cleared.children)throw Error('Clear left duplicate instances or replacements');
 await call('window.signResetDone=false;window.signResetError=null;physicalSigns.reset().then(()=>window.signResetDone=true).catch(e=>window.signResetError=String(e));void 0');
 for(let i=0;i<120;i++){const s=await call('({done:window.signResetDone,error:window.signResetError})');if(s.error)throw Error(s.error);if(s.done)break;await new Promise(r=>setTimeout(r,100));}
 if(!await call('window.signResetDone'))throw Error('Reset timed out');
 const reset=await call('physicalSigns.states()');checkStates(reset,0);
 const after=await call('physicalSigns.replacements()');if(JSON.stringify(before)!==JSON.stringify(after))throw Error('Reset changed replacement identities');
 const positions=reset.map(r=>({id:r.id,position:r.position}));
 const manifest=JSON.parse(await readFile(join(input,'manifest.json'),'utf8'));
 for(const row of manifest.instances){
  const actual=positions.find(p=>p.id==='sign-'+row.target_index).position;
  const expected=[row.world_anchor[0],row.world_anchor[2],-row.world_anchor[1]];
  if(Math.max(...actual.map((v,i)=>Math.abs(v-expected[i])))>.002)throw Error('Native placement changed '+row.target_index);
 }
 const views=[];
 for(const target of [4,5,6,7,8])for(let phase=0;phase<32;phase++)for(const opposite of ([0,8,16,24].includes(phase)?[false,true]:[false])){
  const view=await call(`(()=>{physicalSigns.seek(${phase*2});return physicalSigns.orderedView(${target},${opposite?Math.PI:0})})()`);
  const file=`target-${target}-phase-${phase}-${opposite?'opposite':'native'}.png`;
  const bytes=Buffer.from(view.png.split(',')[1],'base64');await writeFile(join(output,file),bytes);delete view.png;
  for(const kind of ['sign','static'])if(view[kind+'_png']){const extra=Buffer.from(view[kind+'_png'].split(',')[1],'base64');await writeFile(join(output,file.replace('.png','-'+kind+'.png')),extra);view[kind+'_sha256']=hash(extra);delete view[kind+'_png'];}
  views.push({...view,phase,file,sha256:hash(bytes)});
 }
 const afterPins=Object.fromEntries(await Promise.all(protectedFiles.map(async p=>[p,hash(await readFile(p))])));
 if(JSON.stringify(protectedPins)!==JSON.stringify(afterPins))throw Error('Protected files changed');
 const result={status:'PASS private native-order render passes and unchanged runtime; ambient-order composition and visual review pending',
  resource_manifest_sha256:hash(await readFile(join(input,'resources-v1.json'))),manifest_sha256:hash(await readFile(join(input,'manifest.json'))),contract_sha256:hash(await readFile(join(output,'physical-contract.json'))),
  model_sha256:manifest.model.sha256,native_action_authority:{file:nativeManifestPath,sha256:hash(nativeBytes),actions:[0,210,211],frames:32,ticks_per_frame:2},aliases,samples,independent,wrapped,reset,clear:cleared,replaced_targets:after,
  positions,grounding:await call('physicalSigns.grounding()'),views,protected_files:protectedPins,errors:await call('physicalSigns.errors'),
  limits:['Unchanged application MissionStateLayer used with checked HTTP model loader; private fixture is not live catalog publication.',
          'Original static context retained at both camera directions; source ambient loops not newly modeled by this proof.',
          'Physical first-hit and painted-shadow contact review remains necessary.']};
 await writeFile(join(output,'verification.json'),JSON.stringify(result,null,2)+'\n');console.log(JSON.stringify({status:result.status,instances:5,phases:32,views:views.length}));
}catch(error){await writeFile(join(output,'failure.json'),JSON.stringify({error:String(error),logs:logs.slice(-6000)},null,2));throw error}
finally{ws?.close();chrome.kill('SIGTERM');await closed;await vite.close();await writeFile(join(output,'browser.log'),logs)}
