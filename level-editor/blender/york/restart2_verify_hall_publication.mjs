import {spawn} from 'node:child_process';
import {readFile,writeFile,mkdtemp,mkdir,rm} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import {resolve,dirname,join} from 'node:path';
import {chromeEndpoint,socketOpen,evaluate as boundedEvaluate} from '../../app/tests/cdp.mjs';
const evaluate=(socket,id,expression)=>boundedEvaluate(socket,id,expression,{timeoutMs:config.cdp_timeout_ms??60000});
if(!process.argv[2])throw Error('Usage: node verify_publication.mjs config.json [editor URL]');
const configPath=resolve(process.argv[2]);
const here=dirname(configPath),config=JSON.parse(await readFile(configPath,'utf8'));
const base=process.argv[3]??'http://127.0.0.1:5180';
const verifyLive=async()=>{for(const [path,hash]of Object.entries(config.protected_live_files??{})){if(createHash('sha256').update(await readFile(path)).digest('hex')!==hash)throw Error('Live file changed during read-only browser test '+path);}};
await writeFile(join(here,'result.json'),JSON.stringify({status:'RUNNING',phase:'verifying-live-input-hashes'}));
try{await verifyLive();}catch(error){await writeFile(join(here,'result.json'),JSON.stringify({status:'FAIL',phase:'verifying-live-input-hashes',error:String(error)},null,2));throw error;}
console.log('Live hashes verified; starting Chromium');
// Profile root: config.browser_profile_root, else TMPDIR (sandboxed runs), else the historical cache path.
const temporaryRoot=config.browser_profile_root??(process.env.TMPDIR?join(process.env.TMPDIR,'publication-browser'):'/home/phire/.cache/sccache/leicester-browser');await mkdir(temporaryRoot,{recursive:true});
const profile=await mkdtemp(join(temporaryRoot,'p-'));
const chrome=spawn('/usr/lib/chromium/chromium',['--headless','--window-size='+(config.viewport?.width??1500)+','+(config.viewport?.height??1200),'--no-sandbox','--disable-dev-shm-usage','--disable-background-networking','--enable-unsafe-swiftshader','--use-angle=swiftshader','--remote-debugging-port=0','--user-data-dir='+profile,base],{stdio:['ignore','ignore','pipe'],env:{...process.env,TMPDIR:temporaryRoot}});
chrome.stderr.on('data',data=>process.stderr.write(data));
const closed=new Promise(resolve=>chrome.on('close',resolve));let ws,id=0;
try{
 const endpoint=new URL(await chromeEndpoint(chrome,{timeoutMs:15000}));const pages=await(await fetch('http://'+endpoint.host+'/json/list')).json();ws=new WebSocket(pages.find(page=>page.type==='page').webSocketDebuggerUrl);await socketOpen(ws);
 const request=(method,params)=>new Promise((resolve,reject)=>{
  const rid=++id;
  const cleanup=()=>{clearTimeout(timer);ws.removeEventListener('message',message);ws.removeEventListener('close',closed);ws.removeEventListener('error',failed);};
  const done=(error,value)=>{cleanup();error?reject(error):resolve(value);};
  const message=e=>{const data=JSON.parse(e.data);if(data.id===rid)done(data.error,data.result);};
  const closed=()=>done(Error(method+' disconnected'));
  const failed=()=>done(Error(method+' transport failed'));
  const timer=setTimeout(()=>done(Error(method+' timed out')),method==='Page.captureScreenshot'?120000:30000);
  ws.addEventListener('message',message);ws.addEventListener('close',closed);ws.addEventListener('error',failed);
  ws.send(JSON.stringify({id:rid,method,params}));
 });
 // The editor opens its HTTP library (/library/) at startup. Route that library to exactly the
 // hash-pinned staged inputs, before any page script runs, so the unmodified app loads them.
 const pinned=Object.fromEntries(config.files.map(item=>[item.path,{url:item.url,sha256:item.sha256}]));
 await request('Page.enable',{});
 await request('Page.addScriptToEvaluateOnNewDocument',{source:`(()=>{const files=${JSON.stringify(pinned)},maps=${JSON.stringify([config.map+'.rhlos-map.json'])},original=window.fetch.bind(window);
  const digest=async bytes=>Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',bytes)),b=>b.toString(16).padStart(2,'0')).join('');
  window.fetch=async(input,init)=>{const url=new URL(typeof input==='string'||input instanceof URL?String(input):input.url,location.href);
   if(url.origin!==location.origin||!url.pathname.startsWith('/library/'))return original(input,init);
   const path=url.pathname.slice('/library/'.length).split('/').map(decodeURIComponent).join('/');
   if(path==='scenes/index.json')return new Response(JSON.stringify(maps),{headers:{'content-type':'application/json'}});
   const file=files[path];if(!file)return new Response('',{status:404});
   const response=await original(file.url,{cache:'no-store'});if(!response.ok)throw Error('Pinned publication file unreadable: '+path);
   const bytes=await response.arrayBuffer();if(await digest(bytes)!==file.sha256)throw Error('Pinned publication file changed: '+path);
   return new Response(bytes,{headers:{'content-type':response.headers.get('content-type')??'application/octet-stream'}});};})();`});
 await request('Page.reload',{});await new Promise(r=>setTimeout(r,1500));
 const screenshot=async name=>writeFile(join(here,name+'.png'),Buffer.from((await request('Page.captureScreenshot',{format:'png'})).data,'base64'));
 let appReady=false;
 for(let i=0;i<300;i++){
  try{if(await evaluate(ws,++id,"document.querySelector('.app')!==null")){appReady=true;break;}}
  catch(error){if(!/Cannot find default execution context|Execution context was destroyed/.test(String(error)))throw error;}
  await new Promise(r=>setTimeout(r,100));
 }
 if(!appReady)throw Error('Editor application did not become ready');
 await evaluate(ws,++id,'window.__publicationConfig='+JSON.stringify(config));
 await evaluate(ws,++id,await readFile(new URL('./restart2_hall_publication_check.js',import.meta.url),'utf8'));
 let result,phase,lastProgress=0;
 for(let i=0;i<Math.ceil((config.audit_timeout_ms??240000)/200);i++){
  result=await evaluate(ws,++id,'window.__publicationResult');if(result)break;
  if(performance.now()-lastProgress>15000){const progress=await evaluate(ws,++id,'window.__publicationProgress')??{phase:'app-startup'};console.log(JSON.stringify(progress));await writeFile(join(here,'progress.json'),JSON.stringify({status:'RUNNING',...progress}));lastProgress=performance.now();}
  phase=await evaluate(ws,++id,'window.__publicationPhase');if(phase&&!phase.captured){await writeFile(join(here,'progress.json'),JSON.stringify({status:'RUNNING',phase:phase.phase+'-screenshot'}));await new Promise(r=>setTimeout(r,800));await screenshot(phase.phase==='map-ready'?'map-before-insertion':'map-revealed');await evaluate(ws,++id,'window.__publicationPhase.captured=true;window.__publicationContinue=true');}
  await new Promise(r=>setTimeout(r,200));
 }
 if(result?.status!=='PASS'){await screenshot('failure');await writeFile(join(here,'result.json'),JSON.stringify(result??{status:'TIMEOUT'},null,2));throw Error(JSON.stringify(result));}
 if(!config.visual_only){
 await writeFile(join(here,'interaction-result.json'),JSON.stringify(result,null,2));
 const savedResponse=await request('Runtime.evaluate',{expression:`(async()=>{const maps=await(await(await navigator.storage.getDirectory()).getDirectoryHandle('sherwood-level-editor')).getDirectoryHandle('maps');return JSON.parse(await(await(await maps.getFileHandle(${JSON.stringify(config.map+'.rhlos-map.json')})).getFile()).text());})()`,awaitPromise:true,returnByValue:true});
 if(savedResponse.exceptionDetails)throw Error(JSON.stringify(savedResponse.exceptionDetails));
 const stored=savedResponse.result.value;
 await writeFile(join(here,'saved-document.rhlos-map.json'),JSON.stringify(stored,null,2));
 await writeFile(join(here,'progress.json'),JSON.stringify({status:'RUNNING',phase:'saved-document-full-reload'}));
 await request('Page.reload',{});await new Promise(r=>setTimeout(r,1500));
 // The editor does not reopen a map after reload; choose the saved browser copy like a user would.
 const reopen=`(()=>{const card=[...document.querySelectorAll('.map-card-open')].find(button=>button.dataset.map===${JSON.stringify(config.map+' (Modified)')});if(!card||card.disabled)return false;card.click();return true;})()`;
 let reopened=false;for(let i=0;i<300&&!reopened;i++){try{reopened=await evaluate(ws,++id,reopen);}catch(error){if(!/Cannot find default execution context|Execution context was destroyed/.test(String(error)))throw error;}if(!reopened)await new Promise(r=>setTimeout(r,200));}
 if(!reopened)throw Error('Saved publication map copy is not offered after reload');
 let restored=false;for(let i=0;i<Math.ceil((config.reload_timeout_ms??60000)/200);i++){if(await evaluate(ws,++id,`(()=>{const filter=document.querySelector('.shared-library select[aria-label="Source level"]');if(filter&&filter.value!==''){filter.value='';filter.dispatchEvent(new Event('change',{bubbles:true}));}const helpers=[...document.querySelectorAll('.shared-library label')].find(label=>label.textContent.includes('Show gameplay helpers'))?.querySelector('input');if(helpers&&!helpers.checked)helpers.click();return document.querySelectorAll('.object-list li.depth-0').length===${result.savedGroups+(config.expected.ungrouped_parts??0)} && document.querySelectorAll('.shared-library .asset-card button[aria-label^="Add "]').length===${config.expected.assets.length};})()`)){restored=true;break;}await new Promise(r=>setTimeout(r,200));}
 if(!restored){await screenshot('reload-failure');throw Error('Full browser reload did not restore saved publication instances: '+await evaluate(ws,++id,'document.body.innerText'));}
 const hallReloadResponse=await request('Runtime.evaluate',{awaitPromise:true,returnByValue:true,expression:`(async()=>{
 const expected=${JSON.stringify(result.hallBindings??[])};
 const maps=await(await(await navigator.storage.getDirectory()).getDirectoryHandle('sherwood-level-editor')).getDirectoryHandle('maps');
 const stored=JSON.parse(await(await(await maps.getFileHandle('york.rhlos-map.json')).getFile()).text());
 const shared=await import(${JSON.stringify(config.shared_module_url)});
 const {openHttpLibrary}=await import('/src/http-library.ts');const library=(await openHttpLibrary()).handle;
 const {readPinnedAssetDescriptors}=await import('/src/projection-library.ts');
 const expanded=shared.expandStoredMap(stored),descriptors=await readPinnedAssetDescriptors(library,expanded.assetSources??[],expanded.sceneAssets??[]);
 const doc=shared.parseStoredMap(stored,descriptors);
 for(const row of expected){if(JSON.stringify(doc.groups.find(g=>g.id===row.id)?.patches?.['york-castle-great-hall'])!==JSON.stringify(row.bindings))throw Error('Reload changed hall bindings '+row.id);}
 const ids=expected.flatMap(row=>Object.values(row.bindings));
 const labels=()=>[...document.querySelectorAll('.view-settings label')].filter(l=>l.textContent.includes('Reveal interior:'));
 for(const id of ids){const target=()=>labels().find(l=>l.textContent.trim()==='Reveal interior: '+id)?.querySelector('input');if(!target())throw Error('Reload missing hall control '+id);const before=ids.filter(x=>x!==id).map(other=>labels().find(l=>l.textContent.trim()==='Reveal interior: '+other)?.querySelector('input').checked);target().checked=true;target().dispatchEvent(new Event('change',{bubbles:true}));await new Promise(r=>setTimeout(r,70));if(!target().checked)throw Error('Reload toggle failed');const after=ids.filter(x=>x!==id).map(other=>labels().find(l=>l.textContent.trim()==='Reveal interior: '+other)?.querySelector('input').checked);if(JSON.stringify(before)!==JSON.stringify(after))throw Error('Reload shared controls');target().checked=false;target().dispatchEvent(new Event('change',{bubbles:true}));await new Promise(r=>setTimeout(r,70));}
 return {status:'PASS',instances:expected.length,independentControls:ids.length,bindings:expected.map(row=>({id:row.id,bindings:doc.groups.find(g=>g.id===row.id).patches['york-castle-great-hall']})),controls:ids};})()`});
 if(hallReloadResponse.exceptionDetails)throw Error(JSON.stringify(hallReloadResponse.exceptionDetails));
 const hallReload=hallReloadResponse.result?.value;
 if(hallReload?.status!=='PASS'||hallReload.instances!==3||hallReload.independentControls!==6||new Set(hallReload.controls??[]).size!==6||JSON.stringify(hallReload.bindings)!==JSON.stringify(result.hallBindings))throw Error('Missing or inexact awaited hall reload proof: '+JSON.stringify(hallReload));
 result.hallReload=hallReload;
 result.checks.push('full page reload restores saved groups, pinned external models and all palette entries');
 await screenshot('map-after-reload');
 }
 await verifyLive();result.liveFileHashesUnchanged=Object.keys(config.protected_live_files??{}).length;
 await writeFile(join(here,'result.json'),JSON.stringify(result,null,2));console.log(JSON.stringify({status:result.status,groups:result.mapGroups,parts:result.mapParts,assets:result.insertedAssets?.length,visualOnly:result.visualOnly,checks:result.checks}));
}catch(error){await writeFile(join(here,'result.json'),JSON.stringify({status:'FAIL',error:String(error),stack:error.stack},null,2));throw error;}finally{ws?.close();chrome.kill('SIGTERM');await closed;await rm(profile,{recursive:true,force:true});}
