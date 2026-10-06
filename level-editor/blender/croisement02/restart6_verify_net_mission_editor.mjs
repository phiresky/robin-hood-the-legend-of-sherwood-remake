// Verify the staged mission net catalog through the actual editor UI.
import {spawn} from 'node:child_process';
import {readFile,writeFile,mkdtemp,statfs} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import {tmpdir} from 'node:os';
import {createHash} from 'node:crypto';
import {chromeEndpoint,socketOpen} from '../../app/tests/cdp.mjs';

const disk=await statfs('.');if(disk.bavail*disk.bsize<23*1024**3)throw Error('Disk below23GiB');
const root=resolve('.'),base=join(root,'level-editor/work/croisement02-refinement/restart2-state/full-editor-net-variants-v1');
const stage=join(root,'level-editor/work/croisement02-refinement/restart2-state/net-mission-package-v1'),manifest=JSON.parse(await readFile(join(stage,'manifest.json'),'utf8')),staged=new Map([...manifest.files,manifest.private_index].map(r=>['/library/'+r.path,r]));
manifest.entries=JSON.parse(await readFile(join(stage,'entries.json'),'utf8')).map(entry=>({entry,mission:entry.mission}));
const reference=JSON.parse(await readFile(join(root,'level-editor/work/croisement02-refinement/restart2-state/integrating-foreground-private-v1/independent-reference.json'),'utf8'));
let faultPath='',faultMode='';
const runtimeFiles=['level-editor/shared/src/native-state-presentation.ts','level-editor/shared/src/state-delivery.ts','level-editor/app/src/native-state-presentation.ts','level-editor/app/src/state-delivery.ts','level-editor/app/src/StatePreview.tsx','level-editor/app/src/editor-viewport.ts','level-editor/app/src/mission-state-catalog.ts','level-editor/app/src/scene-assets.ts','level-editor/app/src/projection-library.ts'];
const runtimePins=Object.fromEntries(await Promise.all(runtimeFiles.map(async path=>[path,createHash('sha256').update(await readFile(path)).digest('hex')])));
const {createServer:createViteServer}=await import('../../app/node_modules/vite/dist/node/index.js');
const server=await createViteServer({configFile:join(root,'level-editor/app/vite.config.ts'),root:join(root,'level-editor/app'),cacheDir:join(root,'level-editor/work/croisement02-refinement/restart2-state/full-editor-net-variants-v1/vite-cache'),plugins:[{name:'private-state-visibility-audit',enforce:'pre',configureServer(s){s.middlewares.use(async(req,res,next)=>{const path=(req.url??'').split('?')[0];if(!staged.has(path))return next();try{const row=staged.get(path);if(row.path===faultPath){res.statusCode=faultMode==='missing'?404:200;res.setHeader('Cache-Control','no-store');res.end(faultMode==='missing'?'missing':'changed');return;}const bytes=await readFile(join(stage,'library',row.path));res.setHeader('Content-Type',row.path.endsWith('.png')?'image/png':row.path.endsWith('.glb')?'model/gltf-binary':'application/json');res.setHeader('Cache-Control','no-store');res.end(bytes)}catch(e){res.statusCode=500;res.end(String(e))}})},transform(code,id){if(id.split('?')[0].endsWith('/app/src/editor-viewport.ts'))return code+'\nconst auditSetup=EditorViewport.prototype.setup;EditorViewport.prototype.setup=function(...args){window.__stateAuditViewport=this;return auditSetup.apply(this,args)};';}}],server:{watch:null,hmr:false,host:'127.0.0.1',port:0}});await server.listen();
const origin='http://127.0.0.1:'+server.httpServer.address().port,profile=await mkdtemp(join(tmpdir(),'croisement02-full-state-proof-'));
const chrome=spawn('/usr/lib/chromium/chromium',['--headless','--no-sandbox','--disable-dev-shm-usage','--disable-background-networking','--enable-unsafe-swiftshader','--use-angle=swiftshader','--remote-debugging-port=0','--user-data-dir='+profile,'about:blank'],{stdio:['ignore','ignore','pipe']});
const closed=new Promise(r=>chrome.on('close',r));let ws,id=0;
function command(method,params){return new Promise((resolve,reject)=>{const request=++id;const listener=event=>{const data=JSON.parse(event.data);if(data.id===request){ws.removeEventListener('message',listener);data.error?reject(Error(JSON.stringify(data.error))):resolve(data.result);}};ws.addEventListener('message',listener);ws.send(JSON.stringify({id:request,method,params}));});}
try{
 const endpoint=new URL(await chromeEndpoint(chrome));const pages=await(await fetch('http://'+endpoint.host+'/json/list')).json();ws=new WebSocket(pages.find(p=>p.type==='page').webSocketDebuggerUrl);await socketOpen(ws);ws.addEventListener('message',e=>{const r=JSON.parse(e.data);if(['Runtime.exceptionThrown','Runtime.consoleAPICalled','Page.frameNavigated'].includes(r.method))console.log(r.method==='Runtime.exceptionThrown'?JSON.stringify(r.params.exceptionDetails?.exception?.description??r.params.exceptionDetails):r.method)});await command('Runtime.enable');await command('Page.enable');await command('Emulation.setDeviceMetricsOverride',{width:1500,height:1100,deviceScaleFactor:1,mobile:false});
 await command('Page.addScriptToEvaluateOnNewDocument',{source:'window.stateProofFetches=[];'});
 await command('Network.enable');const resources=[];ws.addEventListener('message',e=>{const r=JSON.parse(e.data);if(r.method==='Network.responseReceived'&&r.params.response.url.includes('/library/mission-states/'))resources.push({url:r.params.response.url,status:r.params.response.status,expectedFault:!!faultPath&&r.params.response.url.endsWith(faultPath)})});
 const run=async expression=>{const r=await command('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true});if(r.exceptionDetails)throw Error(JSON.stringify(r.exceptionDetails));return r.result?.value},pause=ms=>new Promise(r=>setTimeout(r,ms));
 const until=async(expr,limit=350)=>{for(let i=0;i<limit;i++){if(await run(expr))return;if(i===100&&expr.includes('map-card-open'))await command('Page.reload',{ignoreCache:true});if(i%100===0)console.log('Waiting '+expr+' '+await run('document.body.innerText.slice(-800)'));await pause(200)}throw Error('Timeout '+expr+' '+await run('document.body.innerText.slice(-3000)'))};
 const shot=async name=>{const r=await command('Page.captureScreenshot',{format:'png'});await writeFile(join(base,name+'.png'),Buffer.from(r.data,'base64'))};
 await command('Page.navigate',{url:origin});await until(`!!document.querySelector('.map-card-open[data-map="croisement02"]')&&!document.querySelector('.map-card-open[data-map="croisement02"]').disabled`);await run(`document.querySelector('.map-card-open[data-map="croisement02"]').click()`);await until(`!!document.querySelector('[data-map-name="croisement02"]')&&!document.querySelector('.map-load-dialog')`);await run(`[...document.querySelectorAll('button')].find(b=>b.textContent==='Mission').click()`);await until(`!!document.querySelector('select[aria-label="Mission"]')`);const checks=[], snapshots=[];
 const check=async(name,expr)=>{const pass=await run(expr);checks.push({name,pass});if(!pass)throw Error(name)};
 let activeMission='';
 for(const entry of manifest.entries){
  const contract=JSON.parse(await readFile(join(stage,'library',entry.entry.contract.path),'utf8')),family=contract.families[0];
  if(activeMission!==entry.mission){await run(`(()=>{const s=document.querySelector('select[aria-label="Mission"]');s.value=${JSON.stringify(entry.mission)};s.dispatchEvent(new Event('change',{bubbles:true}))})()`);activeMission=entry.mission;await until(`!!document.querySelector('[aria-label="State preview asset"] option[value="${entry.entry.id}"]')`)}
  await run(`(()=>{const s=document.querySelector('[aria-label="State preview asset"]');s.value=${JSON.stringify(entry.entry.id)};s.dispatchEvent(new Event('change',{bubbles:true}))})()`);
  await until(`window.__stateAuditViewport.stateDelivery.ready&&window.__stateAuditViewport.stateDelivery.contract.native.mission===${JSON.stringify(contract.native.mission)}&&window.__stateAuditViewport.stateDelivery.contract.families[0].id===${JSON.stringify(family.id)}&&!document.querySelector('[aria-label="State preview"] [role="status"]')`);
  const id=JSON.stringify(family.id);
  await check(entry.entry.id+' exact loaded contract',`JSON.stringify(window.__stateAuditViewport.stateDelivery.contract)===${JSON.stringify(JSON.stringify(contract))}`);
  await check(entry.entry.id+' starts native and paused',`window.__stateAuditViewport.statePresentationMode==='native-art'&&!window.__stateAuditViewport.stateDelivery.native.isPlaying`);
  const golden=reference.records.find(r=>r.contract==='contracts/'+entry.entry.contract.path.split('/').at(-1));
  const pixelHash=`(async()=>Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',window.__stateAuditViewport.stateDelivery.native.pixels().data)),v=>v.toString(16).padStart(2,'0')).join(''))()`;
  await check(entry.entry.id+' exact initial pixels',`${pixelHash}.then(h=>h===${JSON.stringify(golden.cases[0].rgba_sha256)})`);
  for(const tick of [0,Math.floor(family.body_terminal_tick/2),family.body_terminal_tick]){
   await run(`window.__stateAuditViewport.stateDelivery.native.seek(${tick});window.__stateAuditViewport.seekDeliveredState(${id},${tick})`);
   await check(entry.entry.id+' exact pixels '+tick,`${pixelHash}.then(h=>h===${JSON.stringify(golden.cases.find(c=>c.tick===tick).rgba_sha256)})`);
   await check(entry.entry.id+' seek '+tick,`window.__stateAuditViewport.deliveredStateStatus(${id}).tick===${tick}`);
  }
  await run(`(()=>{const input=document.querySelector('[aria-label="State preview frame"]');input.value='${family.body_terminal_tick}';input.dispatchEvent(new Event('input',{bubbles:true}));[...document.querySelectorAll('[aria-label="State preview"] button')].find(b=>b.textContent==='Play').click()})()`);
  await until(`window.__stateAuditViewport.deliveredStateStatus(${id}).tick>${family.body_terminal_tick+3}`);
  await check(entry.entry.id+' final loop stays playing',`window.__stateAuditViewport.deliveredStateStatus(${id}).playing`);
  await run(`[...document.querySelectorAll('[aria-label="State preview"] button')].find(b=>b.textContent==='Pause').click()`);
  await check(entry.entry.id+' final loop pauses',`!window.__stateAuditViewport.deliveredStateStatus(${id}).playing`);
  if(entry.mission==='Emb05_FoB_MP'){await shot(entry.entry.id+'-native');snapshots.push(entry.entry.id+'-native.png')}
  for(const endpoint of ['initial','applied']){
   await run(`(()=>{const s=document.querySelector('[aria-label="State preview view"]');s.value='${endpoint}';s.dispatchEvent(new Event('change',{bubbles:true}))})()`);
   await check(entry.entry.id+' '+endpoint+' endpoint',`(()=>{const d=window.__stateAuditViewport.stateDelivery,p=d.roots.get(${id});return d.mode==='physical-endpoint'&&d.physical.visible&&p.${endpoint}.visible&&!p.${endpoint==='initial'?'applied':'initial'}.visible&&p.${endpoint}.children.length===${family.physical[endpoint].length}})()`);
   if(entry.mission==='Emb05_FoB_MP'&&['net-01-occupied','net-03-empty'].includes(family.id)){await shot(entry.entry.id+'-'+endpoint);snapshots.push(entry.entry.id+'-'+endpoint+'.png')}
  }
  await run(`(()=>{const s=document.querySelector('[aria-label="State preview view"]');s.value='art';s.dispatchEvent(new Event('change',{bubbles:true}))})()`);
  await run(`window.__stateAuditViewport.stateDelivery.native.seek(0);window.__stateAuditViewport.resetDeliveredState(${id})`);
  await check(entry.entry.id+' exact reset pixels',`${pixelHash}.then(h=>h===${JSON.stringify(golden.cases[0].rgba_sha256)})`);
  await check(entry.entry.id+' reset restores initial source',`window.__stateAuditViewport.deliveredStateStatus(${id}).tick===undefined&&!window.__stateAuditViewport.stateDelivery.physical.visible`);
 }
 await run(`(()=>{const s=document.querySelector('select[aria-label="Mission"]');s.value='';s.dispatchEvent(new Event('change',{bubbles:true}))})()`);
 await until(`!document.querySelector('[aria-label="State preview view"]')`);
 await check('Map-only disposes every staged family',`!window.__stateAuditViewport.stateDelivery.ready&&!window.__stateAuditViewport.nativeArt.ready&&window.__stateAuditViewport.stateDelivery.physical.children.length===0&&window.__stateAuditViewport.statePresentationMode==='physical'`);
 const first=manifest.entries[0],firstContract=JSON.parse(await readFile(join(stage,'library',first.entry.contract.path),'utf8'));
 for(const mode of ['missing','changed']){
  faultPath=firstContract.native.patch_states[0].profile.path;faultMode=mode;
  await run(`(()=>{const s=document.querySelector('select[aria-label="Mission"]');s.value=${JSON.stringify(first.mission)};s.dispatchEvent(new Event('change',{bubbles:true}))})()`);
  await until(`!!document.querySelector('[aria-label="State preview asset"] option[value="${first.entry.id}"]')`);
  await run(`(()=>{const s=document.querySelector('[aria-label="State preview asset"]');s.value=${JSON.stringify(first.entry.id)};s.dispatchEvent(new Event('change',{bubbles:true}))})()`);
  await until(`!!document.querySelector('[aria-label="State preview"] [role="status"]')`);
  await check(mode+' declared source fails atomically',`!window.__stateAuditViewport.stateDelivery.ready&&window.__stateAuditViewport.stateDelivery.physical.children.length===0`);
  await run(`(()=>{const s=document.querySelector('select[aria-label="Mission"]');s.value='';s.dispatchEvent(new Event('change',{bubbles:true}))})()`);
  await until(`!document.querySelector('[aria-label="State preview view"]')`);faultPath='';faultMode='';
 }
 const hashes=[];for(const row of [...manifest.files,manifest.private_index]){const response=await fetch(origin+'/library/'+row.path),bytes=Buffer.from(await response.arrayBuffer()),actual=createHash('sha256').update(bytes).digest('hex');if(!response.ok||actual!==row.sha256)throw Error('Staged bytes differ '+row.path);hashes.push({path:row.path,sha256:actual})}
 if(resources.some(r=>r.status!==200&&!r.expectedFault))throw Error('State HTTP resource failure');
 for(const [path,digest] of Object.entries(runtimePins))if(createHash('sha256').update(await readFile(path)).digest('hex')!==digest)throw Error('Runtime changed during proof '+path);
 await writeFile(join(base,'verification.json'),JSON.stringify({status:'PASS',runtimePins,checks,staged_hashes:hashes,snapshots,resources,manifest_sha256:createHash('sha256').update(await readFile(join(stage,'manifest.json'))).digest('hex'),scope:'Actual Editor/MissionPanel with private staged contract/art/model HTTP overlay and existing installed resource bytes. No canonical publication or full refined scene claim.'},null,2)+'\n');console.log('PASS '+checks.length+' actual Editor staged mission checks');
}catch(error){try{const r=await command('Page.captureScreenshot',{format:'png'});await writeFile(join(base,'failure.png'),Buffer.from(r.data,'base64'))}catch{}await writeFile(join(base,'failure.json'),JSON.stringify({error:String(error)},null,2));throw error}finally{ws?.close();chrome.kill('SIGTERM');await closed;await server.close();console.log('Profile retained '+profile)}
