import {stripTypeScriptTypes} from 'node:module';
import {createServer} from 'node:http';
import {spawn} from 'node:child_process';
import {readFile,writeFile,mkdtemp,rm} from 'node:fs/promises';
import {resolve,extname,join} from 'node:path';
import {tmpdir} from 'node:os';
import {createHash} from 'node:crypto';
import {chromeEndpoint,socketOpen,evaluate} from '/home/phire/data/dev/2026/robin-hood-the-legend-of-sherwood/level-editor/app/tests/cdp.mjs';

const root=resolve('.'),base=join(root,'level-editor/work/croisement02-refinement/restart2-state/remaining-seven-contact-v1');
const {createServer:createViteServer}=await import('/home/phire/data/dev/2026/robin-hood-the-legend-of-sherwood/level-editor/app/node_modules/vite/dist/node/index.js');
const {default:solid}=await import('/home/phire/data/dev/2026/robin-hood-the-legend-of-sherwood/level-editor/app/node_modules/@solidjs/vite-plugin/dist/esm/index.mjs');
const server=await createViteServer({plugins:[solid({solid:{delegateEvents:false}})],configFile:false,root:join(root,'level-editor/app'),cacheDir:join(base,'vite-cache'),optimizeDeps:{entries:[]},resolve:{alias:[{find:/^three\/addons\//,replacement:join(root,'level-editor/app/node_modules/three/examples/jsm/')+'/'},{find:/^three$/,replacement:join(root,'level-editor/app/node_modules/three/build/three.module.js')},{find:/^three\//,replacement:join(root,'level-editor/app/node_modules/three/')+'/' }]},server:{watch:null,hmr:false,host:'127.0.0.1',port:0,fs:{allow:[root]}}});await server.listen();
const origin='http://127.0.0.1:'+server.httpServer.address().port,profile=await mkdtemp(join(tmpdir(),'croisement02-loader-proof-'));
const chrome=spawn('/usr/lib/chromium/chromium',['--headless','--no-sandbox','--disable-dev-shm-usage','--disable-background-networking','--enable-unsafe-swiftshader','--use-angle=swiftshader','--remote-debugging-port=0','--user-data-dir='+profile,'about:blank'],{stdio:['ignore','ignore','pipe']});
const closed=new Promise(r=>chrome.on('close',r));let ws,id=0;
function command(method,params){return new Promise((resolve,reject)=>{const request=++id;const listener=event=>{const data=JSON.parse(event.data);if(data.id===request){ws.removeEventListener('message',listener);data.error?reject(Error(JSON.stringify(data.error))):resolve(data.result);}};ws.addEventListener('message',listener);ws.send(JSON.stringify({id:request,method,params}));});}
try{
 const endpoint=new URL(await chromeEndpoint(chrome));const pages=await(await fetch('http://'+endpoint.host+'/json/list')).json();ws=new WebSocket(pages.find(p=>p.type==='page').webSocketDebuggerUrl);await socketOpen(ws);await command('Runtime.enable');await command('Page.navigate',{url:origin+'/@fs/'+join(base,'index.html')});const run=e=>evaluate(ws,++id,e);let ready;
 const errors=[];ws.addEventListener('message',e=>{const d=JSON.parse(e.data);if(d.method==='Runtime.exceptionThrown')errors.push(d.params)});
 for(let i=0;i<1200;i++){ready=await run('!!window.stateContactProof?.ready');if(ready)break;if(errors.length)throw Error(JSON.stringify(errors));await new Promise(r=>setTimeout(r,100))}
 if(!ready)throw Error('Focused contact fixture failed to load');
 const checks=await run('stateContactProof.checks'),views=[];
 for(let i=0;i<4;i++)for(const endpoint of ['initial','applied'])for(const view of [0,1]){const result=await run(`stateContactProof.show(${i},${JSON.stringify(endpoint)},${view})`);const png=await run('stateContactProof.capture()'),name=`${i}-${endpoint}-${view}.png`,bytes=Buffer.from(png.split(',')[1],'base64');await writeFile(join(base,name),bytes);views.push({...result,image:name,sha256:createHash('sha256').update(bytes).digest('hex')})}
 if(errors.length)throw Error(JSON.stringify(errors));
 await writeFile(join(base,'verification.json'),JSON.stringify({status:'RENDERED_FOR_VISUAL_REVIEW',checks,views,manifest_sha256:createHash('sha256').update(await readFile(join(base,'manifest.json'))).digest('hex'),scope:'Exact same physical endpoint and contact receiver bytes. Native direction first; reverse view supplementary. Foliage, actors and unrelated static scenery intentionally absent; not full-scene occlusion proof.'},null,2)+'\n');console.log('Rendered16 bounded contact views');

}finally{ws?.close();chrome.kill('SIGTERM');await closed;await server.close();console.log('Retained profile '+profile);}
