import {spawn} from 'node:child_process';
import {mkdtemp,writeFile,rm} from 'node:fs/promises';
import {resolve} from 'node:path';
import {chromeEndpoint,socketOpen,evaluate} from '../../app/tests/cdp.mjs';
const live=process.argv.includes('--live');
const base=resolve('level-editor/work/croisement01-refinement/restart2/approved-integration-batch-v2');
const {createServer}=await import('../../app/node_modules/vite/dist/node/index.js');
const server=await createServer({root:resolve('level-editor/app'),configFile:resolve('level-editor/app/vite.config.ts'),cacheDir:base+'/browser-vite-cache',optimizeDeps:{entries:[]},server:{host:'127.0.0.1',port:0}});await server.listen();
const origin='http://127.0.0.1:'+server.httpServer.address().port;
const verificationAssets=live?resolve('level-editor/library/3d-assets'):base+'/assets';
const originalScene=live?base+'/publication-backup-v1/croisement01.rhlos-map.json':resolve('level-editor/library/scenes/croisement01.rhlos-map.json');
const outputPrefix=live?'live-browser':'browser-export';
const profile=await mkdtemp('/home/phire/.cache/crois01-approved-batch-');
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
 for(const [asset,parts] of [['croisement01-small-bank-stones',1],['croisement01-tree-20',2],['croisement01-southeast-small-stump',1]]){const checked=await runAsync(`assetSceneVerification.verify(${JSON.stringify(asset)})`);if(checked.parts!==parts)throw Error('Wrong exact part count for '+asset);const capture=await command('Page.captureScreenshot',{format:'png'});await writeFile(base+'/'+asset+'-'+outputPrefix+'.png',Buffer.from(capture.data,'base64'));const preview=await runAsync(`assetSceneVerification.preview(${JSON.stringify(asset)})`);result.push({asset,...checked,preview});}
 const mapProof=await runAsync(`(async()=>{const {parseStoredMap}=await import('/@fs/'+${JSON.stringify(resolve('level-editor/shared/src/index.ts'))});const stage=await(await fetch('/@fs/'+${JSON.stringify(base+'/croisement01.rhlos-map.json')})).json();const original=await(await fetch('/@fs/'+${JSON.stringify(originalScene)})).json();async function parse(saved){const descriptors=new Map();for(const ref of [...saved.assetSources,...saved.sceneAssets]){const path=['croisement01-small-bank-stones','croisement01-tree-20','croisement01-southeast-small-stump'].includes(ref.id)?(${JSON.stringify(live?resolve('level-editor/library/3d-assets/croisement01'):base+'/assets')}+'/'+ref.id+'/asset.json'):${JSON.stringify(resolve('level-editor/library'))}+'/'+ref.descriptor;descriptors.set(ref.id,await(await fetch('/@fs/'+path)).json());}return parseStoredMap(saved,descriptors);}const a=await parse(original),b=await parse(stage);if(b.objects.length!==a.objects.length+1||b.groups.length!==a.groups.length)throw Error('Approved batch group/part counts differ');return {originalParts:a.objects.length,stagedParts:b.objects.length,originalGroups:a.groups.length,stagedGroups:b.groups.length};})()`);
 const screenshot=await command('Page.captureScreenshot',{format:'png'});await writeFile(base+'/'+outputPrefix+'.png',Buffer.from(screenshot.data,'base64'));
 const preview=result.map(r=>r.preview);
 const memory=await evaluate(ws,++id,'assetSceneVerification.retire()');
 await writeFile(base+'/'+outputPrefix+'-proof.json',JSON.stringify({status:'PASS',assets,result,preview,memory,mapProof,palette_insert_save_reload:true},null,2)+'\n');console.log(JSON.stringify({status:'PASS',result}));
}finally{ws?.close();chrome.kill('SIGTERM');await closed;await server.close();await rm(profile,{recursive:true,force:true});}
