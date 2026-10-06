import {createHash} from 'node:crypto';
import {spawn} from 'node:child_process';
import {mkdtemp,writeFile,readFile,rm} from 'node:fs/promises';
import {resolve} from 'node:path';
import {chromeEndpoint,socketOpen,evaluate} from '../../app/tests/cdp.mjs';
const live=process.argv.includes('--live');
if(!live)throw Error('This bounded verifier requires installed --live scope');
const base=resolve('level-editor/work/croisement03-refinement/restart2/tree25-cluster-integration-v2/stage-v1');
const origin=process.argv.find(value=>value.startsWith('http://'))??'http://127.0.0.1:5180';
const verificationAssets=live?resolve('level-editor/library/3d-assets'):base+'/assets';
const outputPrefix=live?'live-browser':'browser-export';
const publication=JSON.parse(await readFile(base+'/promotion.json','utf8'));
const library=resolve('level-editor/library');
const digest=async path=>createHash('sha256').update(await readFile(path)).digest('hex');
const verifyInstalled=async()=>{
 if(publication.status!=='APPLIED')throw Error('Publication is not applied');
 for(const item of publication.files){
  let actual;try{actual=await digest(item.target);}catch(error){if(error.code!=='ENOENT')throw error;actual=null;}
  if(actual!==item.source_sha256)throw Error('Installed bytes changed: '+item.target);
 }
};
await verifyInstalled();
const profile=await mkdtemp('/home/phire/.cache/crois03-approved-tree25-');
const chrome=spawn('/usr/lib/chromium/chromium',['--headless','--no-sandbox','--disable-dev-shm-usage','--disable-background-networking','--enable-unsafe-swiftshader','--use-angle=swiftshader','--remote-debugging-port=0','--user-data-dir='+profile,origin+'/tests/asset-scenes.html'],{stdio:['ignore','ignore','pipe']});
const closed=new Promise(r=>chrome.on('close',r));let ws,id=0;
const command=(method,params)=>new Promise((resolve,reject)=>{const n=++id;const f=e=>{const m=JSON.parse(e.data);if(m.id===n){ws.removeEventListener('message',f);m.error?reject(Error(JSON.stringify(m.error))):resolve(m.result);}};ws.addEventListener('message',f);ws.send(JSON.stringify({id:n,method,params}));});
try{
 const endpoint=new URL(await chromeEndpoint(chrome));const pages=await(await fetch('http://'+endpoint.host+'/json/list')).json();ws=new WebSocket(pages.find(p=>p.type==='page').webSocketDebuggerUrl);await socketOpen(ws);
 for(let i=0;i<200;i++){try{if(await evaluate(ws,++id,'!!window.assetSceneVerification'))break;}catch(error){if(!String(error).includes('execution context'))throw error;}await new Promise(r=>setTimeout(r,100));}
 await new Promise(r=>setTimeout(r,1500));
 const runAsync=async expression=>{const result=await command('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true});if(result.exceptionDetails)throw Error(JSON.stringify(result.exceptionDetails));return result.result.value;};
 const assets=await runAsync(`assetSceneVerification.configure(${JSON.stringify(verificationAssets)})`);
 const result=[];
 for(const [asset,parts] of [['croisement03-tree-25',1]]){const checked=await runAsync(`assetSceneVerification.verify(${JSON.stringify(asset)})`);if(checked.parts!==parts)throw Error('Wrong exact part count for '+asset);const capture=await command('Page.captureScreenshot',{format:'png'});await writeFile(base+'/'+asset+'-'+outputPrefix+'.png',Buffer.from(capture.data,'base64'));const preview=await runAsync(`assetSceneVerification.preview(${JSON.stringify(asset)})`);result.push({asset,...checked,preview});}
 const mapProof=await runAsync(`(async()=>{const {parseStoredMap}=await import('/@fs/'+${JSON.stringify(resolve('level-editor/shared/src/index.ts'))});const saved=await(await fetch('/@fs/'+${JSON.stringify(resolve('level-editor/library/scenes/croisement03.rhlos-map.json'))})).json();const descriptors=new Map();for(const ref of [...saved.assetSources,...saved.sceneAssets]){descriptors.set(ref.id,await(await fetch('/@fs/'+${JSON.stringify(library)}+'/'+ref.descriptor)).json());}const parsed=parseStoredMap(saved,descriptors);if(parsed.objects.length!==121||parsed.groups.length!==89)throw Error('Installed map counts differ');return {parts:parsed.objects.length,groups:parsed.groups.length};})()`);
 const screenshot=await command('Page.captureScreenshot',{format:'png'});await writeFile(base+'/'+outputPrefix+'.png',Buffer.from(screenshot.data,'base64'));
 const preview=result.map(r=>r.preview);
 const memory=await evaluate(ws,++id,'assetSceneVerification.retire()');
 await verifyInstalled();
 await writeFile(base+'/'+outputPrefix+'-proof.json',JSON.stringify({status:'PASS',assets,result,preview,memory,mapProof,normal_http:true,full_editor_insert_save_reload_proof:base+'/browser-preparation-v1/result.json'},null,2)+'\n');console.log(JSON.stringify({status:'PASS',result}));
}finally{ws?.close();chrome.kill('SIGTERM');await closed;await rm(profile,{recursive:true,force:true});}
