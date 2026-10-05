/** Serial isolated asset loads avoid accumulating complete map texture sets. */
import {spawn} from 'node:child_process';
import {mkdir,mkdtemp,writeFile,rm,readFile} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import {chromeEndpoint,socketOpen} from '../app/tests/cdp.mjs';
const library=resolve(process.argv[2]),output=resolve(process.argv[3]);
const ids=process.argv[4]==='--all'?(JSON.parse(await readFile(join(library,'index.json'),'utf8')).assets.map(entry=>entry.id)):process.argv.slice(4);
if(!ids.length)throw Error('Pass library root, output directory and exact asset IDs');
await mkdir(output,{recursive:true});const profile=await mkdtemp(join(output,'browser-profile-'));
const chrome=spawn('/usr/lib/chromium/chromium',['--headless','--no-sandbox','--disable-dev-shm-usage','--disable-background-networking','--window-size=920,760','--remote-debugging-port=0','--user-data-dir='+profile,'http://localhost:5180/tests/asset-scenes.html'],{stdio:['ignore','ignore','pipe'],env:{...process.env,TMPDIR:'/tmp'}});
let log='',ws,sequence=0;chrome.stderr.on('data',data=>log+=String(data));const closed=new Promise(resolve=>chrome.on('close',resolve));
const result={status:'FAIL',library,serial:true,results:[]};
async function command(method,params={}){const id=++sequence;return new Promise((resolve,reject)=>{const timer=setTimeout(()=>{ws.removeEventListener('message',listener);reject(Error(method+' timeout'));},60000);const listener=event=>{const message=JSON.parse(event.data);if(message.id!==id)return;clearTimeout(timer);ws.removeEventListener('message',listener);message.error?reject(Error(JSON.stringify(message.error))):resolve(message.result);};ws.addEventListener('message',listener);ws.send(JSON.stringify({id,method,params}));});}
async function evaluate(expression){const response=await command('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true});if(response.exceptionDetails)throw Error(JSON.stringify(response.exceptionDetails));return response.result?.value;}
async function capture(id,suffix){const response=await command('Page.captureScreenshot',{format:'png'});await writeFile(join(output,id+'-'+suffix+'.png'),Buffer.from(response.data,'base64'));}
try{
 const endpoint=new URL(await chromeEndpoint(chrome,{timeoutMs:30000}));const pages=await(await fetch('http://'+endpoint.host+'/json/list')).json();ws=new WebSocket(pages.find(page=>page.type==='page').webSocketDebuggerUrl);await socketOpen(ws);await command('Page.enable');
 for(let attempt=0;attempt<120;attempt++){if(await evaluate('!!window.assetSceneVerification'))break;if(attempt===119)throw Error('Fixture failed to load');await new Promise(resolve=>setTimeout(resolve,250));}
 const baseline=await evaluate('window.assetSceneVerification.resourceBaseline()');
 if(baseline.warmed.geometries!==0)throw Error('Renderer warmup leaked geometry');
 const matchesBaseline=memory=>memory.geometries===baseline.warmed.geometries&&memory.textures===baseline.warmed.textures;
 const leakProbe=await evaluate('window.assetSceneVerification.resourceLeakProbe()');
 if(matchesBaseline(leakProbe.leaked))throw Error('Retirement check failed to reject deliberately retained asset texture');
 if(!matchesBaseline(leakProbe.cleaned))throw Error('Negative leak probe did not clean up');
 result.resourceBaseline=baseline;result.negativeLeakCheck={rejected:true,...leakProbe};
 const listed=await evaluate(`window.assetSceneVerification.configure(${JSON.stringify(library)})`);
 const expanded=listed.filter(entry=>ids.some(id=>entry.id===id||entry.id.startsWith(id+'--state-')));
 if(!ids.every(id=>expanded.some(entry=>entry.id===id||entry.id.startsWith(id+'--state-'))))throw Error('Missing requested asset');
 for(const entry of expanded){const record=await evaluate(`window.assetSceneVerification.verify(${JSON.stringify(entry.id)})`);await capture(entry.id,'model');record.preview=await evaluate(`window.assetSceneVerification.preview(${JSON.stringify(entry.id)})`);await capture(entry.id,'preview');record.retired=await evaluate('window.assetSceneVerification.retire()');if(!matchesBaseline(record.retired))throw Error('GPU resources leaked '+entry.id+JSON.stringify(record.retired));result.results.push(record);await writeFile(join(output,'progress.json'),JSON.stringify(result,null,2));}
 result.status='PASS';
}catch(error){result.error=String(error);result.browserLog=log;process.exitCode=1;}
finally{await writeFile(join(output,'report.json'),JSON.stringify(result,null,2));console.log(JSON.stringify({status:result.status,count:result.results.length,error:result.error}));ws?.close();chrome.kill('SIGTERM');await closed;await rm(profile,{recursive:true,force:true});}
