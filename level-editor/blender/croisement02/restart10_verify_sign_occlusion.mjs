/** Private five-instance proof using the application's unchanged state layer. */
import {spawn} from 'node:child_process';
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import {createHash} from 'node:crypto';
import {createServer} from '../../app/node_modules/vite/dist/node/index.js';
import {chromeEndpoint,socketOpen,evaluate} from '../../app/tests/cdp.mjs';

const repo=resolve('.'),root=resolve('level-editor/work/croisement02-refinement/restart10-physical-signs');
const input=join(root,'input-occlusion-v1'),output=join(root,'occlusion-v1');
await mkdir(output);await mkdir(join(output,'profile'));
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
const protectedFiles=['level-editor/library/mission-states/index.json','level-editor/library/scenes/croisement02.rhlos-map.json',
 'level-editor/app/src/mission-state-layer.ts','level-editor/app/src/state-appearance-player.ts'];
const protectedPins=Object.fromEntries(await Promise.all(protectedFiles.map(async p=>[p,hash(await readFile(p))])));
const html=`<!doctype html><meta charset="utf-8"><title>Private physical sign proof</title><style>body{margin:0;background:#323232}</style><script type="module" src="/@fs/${repo}/level-editor/blender/croisement02/restart10_sign_occlusion_viewer.ts"></script>`;
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
 const manifest=JSON.parse(await readFile(join(input,'manifest.json'),'utf8'));const views=[];
 for(const target of [5,8]){
  const names=['none','all',...manifest.context.filter(r=>r.near_targets.includes(target)&&r.source.role!=='ground').map(r=>r.id)];
  for(const omit of names){
   const result=await call(`physicalSigns.omission(${target},${JSON.stringify(omit)})`);
   const file=`target-${target}-omit-${omit}.png`;const bytes=Buffer.from(result.png.split(',')[1],'base64');delete result.png;
   await writeFile(join(output,file),bytes);views.push({...result,file,sha256:hash(bytes)});
  }
 }
 const afterPins=Object.fromEntries(await Promise.all(protectedFiles.map(async p=>[p,hash(await readFile(p))])));
 if(JSON.stringify(protectedPins)!==JSON.stringify(afterPins))throw Error('Protected files changed');
 await writeFile(join(output,'report.json'),JSON.stringify({status:'Read-only occluder omission diagnosis; all approved geometry/materials retained',model_sha256:manifest.model.sha256,manifest_sha256:hash(await readFile(join(input,'manifest.json'))),resource_manifest_sha256:hash(await readFile(join(input,'resources-v1.json'))),views,protected_files:protectedPins},null,2)+'\n');
 console.log(JSON.stringify({status:'Diagnostic rendered',views:views.length}));
}catch(error){await writeFile(join(output,'failure.json'),JSON.stringify({error:String(error),logs:logs.slice(-6000)},null,2));throw error}
finally{ws?.close();chrome.kill('SIGTERM');await closed;await vite.close();await writeFile(join(output,'browser.log'),logs)}
