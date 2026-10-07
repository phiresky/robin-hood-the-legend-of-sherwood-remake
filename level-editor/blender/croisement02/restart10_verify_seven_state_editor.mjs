// Read-only production Editor verification with an immutable private state overlay.
import {readFile,writeFile,mkdir,mkdtemp} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import {pathToFileURL} from 'node:url';
import {tmpdir} from 'node:os';
import {spawn} from 'node:child_process';
import {createHash} from 'node:crypto';
import {chromeEndpoint,socketOpen} from '../../app/tests/cdp.mjs';
import {stateProofCommand,run as states} from './restart6_verify_remaining_state_editor.mjs';
const root=resolve('.'),base=resolve(process.argv[2]||''),out=join(base,'browser');
if(!process.argv[2])throw Error('Expected frozen proof directory');
const inputBytes=await readFile(join(base,'inputs.json')),inputs=JSON.parse(inputBytes),sha=b=>createHash('sha256').update(b).digest('hex');
async function verifyPins(){for(const [path,expected]of Object.entries(inputs.files)){if(sha(await readFile(join(root,path)))!==expected)throw Error('Input changed: '+path)}}
await verifyPins();await mkdir(out,{recursive:false});
const {createServer}=await import('../../app/node_modules/vite/dist/node/index.js');
const {default:solid}=await import('../../app/node_modules/@solidjs/vite-plugin/dist/esm/index.mjs');
const html='<html><body style="margin:0"><div id="result" style="position:fixed;z-index:9999;top:0;right:0;background:#fff;color:#000">STARTING</div><div id="root"></div><script type="module" src="/@fs'+join(base,'editor.tsx')+'"></script></body></html>';
const server=await createServer({configFile:false,root:join(root,'level-editor/app'),cacheDir:join(out,'vite-cache'),plugins:[solid({solid:{delegateEvents:false}}),{name:'frozen-seven-state-proof',configureServer(s){s.middlewares.use((req,res,next)=>{if(req.url!=='/seven-state-proof')return next();res.setHeader('Content-Type','text/html');res.end(html)})}}],resolve:{alias:[{find:/^three\/addons\//,replacement:join(root,'level-editor/app/node_modules/three/examples/jsm/')+'/'},{find:/^three$/,replacement:join(root,'level-editor/app/node_modules/three/build/three.module.js')},{find:/^three\//,replacement:join(root,'level-editor/app/node_modules/three/')+'/'}]},optimizeDeps:{entries:[]},server:{host:'127.0.0.1',port:0,watch:null,hmr:false,fs:{allow:[root]}}});
await server.listen();await server.watcher.close();
const origin='http://127.0.0.1:'+server.httpServer.address().port,profile=await mkdtemp(join(tmpdir(),'c02-seven-installed-'));
const chrome=spawn('/usr/lib/chromium/chromium',['--headless','--no-sandbox','--disable-dev-shm-usage','--disable-background-networking','--enable-unsafe-swiftshader','--use-angle=swiftshader','--remote-debugging-port=0','--user-data-dir='+profile,'about:blank'],{stdio:['ignore','ignore','pipe']});
const exited=new Promise(r=>chrome.on('close',r));let ws,id=0;const events=[];const started=Date.now();
try{
 const endpoint=new URL(await chromeEndpoint(chrome)),pages=await(await fetch('http://'+endpoint.host+'/json/list')).json();ws=new WebSocket(pages.find(p=>p.type==='page').webSocketDebuggerUrl);await socketOpen(ws);
 ws.addEventListener('message',e=>{const d=JSON.parse(e.data);if(['Runtime.exceptionThrown','Inspector.detached','Network.loadingFailed'].includes(d.method))events.push({elapsed_ms:Date.now()-started,...d})});
 const nextId=()=>++id,command=(method,params)=>stateProofCommand(ws,nextId,method,params,180000),evaluate=async expression=>{const r=await command('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true});if(r.exceptionDetails)throw Error(JSON.stringify(r.exceptionDetails));return r.result?.value};
 await command('Runtime.enable');await command('Page.enable');await command('Network.enable');await command('Emulation.setDeviceMetricsOverride',{width:1440,height:1000,deviceScaleFactor:1,mobile:false});
 // Navigation is separate from evaluation so context replacement is expected only here.
 await command('Page.navigate',{url:origin+'/seven-state-proof'});
 let ready=false;const deadline=Date.now()+600000;
 while(Date.now()<deadline){await new Promise(r=>setTimeout(r,500));const status=await evaluate('document.querySelector("#result")?.textContent');if(status?.startsWith('FAIL'))throw Error(status);if(status==='READY'){ready=true;break}}
 if(!ready)throw Error('Installed Editor did not reach READY in ten minutes');
 await writeFile(join(out,'loaded.json'),JSON.stringify({status:'READY',elapsed_ms:Date.now()-started,static_map_sha256:inputs.static_map_sha256},null,2)+'\n');
 const stateResult=await states({ws,nextId,evaluate,out:join(out,'remaining-seven-states')});
 const contacts=await import(pathToFileURL(join(base,'capture.mjs')).href);
 const contactResult=await contacts.run({ws,nextId,origin,out:join(out,'focused-state-contacts')});
 await verifyPins();
 await writeFile(join(out,'verification.json'),JSON.stringify({status:'PASS',inputs_sha256:sha(inputBytes),static_map_sha256:inputs.static_map_sha256,state_result:stateResult,contact_status:contactResult.status,contact_views:contactResult.views.length,source_files_unchanged:Object.keys(inputs.files).length,events,elapsed_ms:Date.now()-started,scope:'Current runtime on actual installed static scene, seven private entries only. Contact images require visual review. No publication.'},null,2)+'\n');console.log('PASS seven states and sixteen contact views');
}catch(error){await writeFile(join(out,'failure.json'),JSON.stringify({error:String(error),events,elapsed_ms:Date.now()-started},null,2)+'\n');throw error}finally{ws?.close();chrome.kill('SIGTERM');await exited;await server.close()}
