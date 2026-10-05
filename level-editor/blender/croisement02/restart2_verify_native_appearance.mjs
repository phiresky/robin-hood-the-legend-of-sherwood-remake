import {createServer} from 'node:http';
import {spawn} from 'node:child_process';
import {readFile,writeFile,mkdtemp,rm} from 'node:fs/promises';
import {resolve,extname,join} from 'node:path';
import {tmpdir} from 'node:os';
import {createHash} from 'node:crypto';
import {chromeEndpoint,socketOpen,evaluate} from '../../app/tests/cdp.mjs';

const root=resolve('.'),base=join(root,'level-editor/work/croisement02-refinement/restart2-state');
const server=createServer(async(req,res)=>{try{
  const pathname=decodeURIComponent(new URL(req.url,'http://localhost').pathname);
  if(!pathname.startsWith('/@fs/'))throw Error('Unsupported route');
  const file=resolve(pathname.slice(5));
  if(!file.startsWith(root+'/'))throw Error('Outside proof repository');
  const types={'.html':'text/html','.js':'text/javascript','.mjs':'text/javascript','.json':'application/json','.png':'image/png','.glb':'model/gltf-binary'};
  res.writeHead(200,{'Content-Type':types[extname(file)]||'application/octet-stream'});res.end(await readFile(file));
}catch(error){res.writeHead(404);res.end(String(error));}});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const origin='http://127.0.0.1:'+server.address().port,profile=await mkdtemp(join(tmpdir(),'croisement02-native-proof-'));
const chrome=spawn('/usr/lib/chromium/chromium',['--headless','--no-sandbox','--disable-dev-shm-usage','--disable-background-networking','--enable-unsafe-swiftshader','--use-angle=swiftshader','--remote-debugging-port=0','--user-data-dir='+profile,'about:blank'],{stdio:['ignore','ignore','pipe']});
const closed=new Promise(r=>chrome.on('close',r));let ws,id=0;
function command(method,params){return new Promise((resolve,reject)=>{const request=++id;const listener=event=>{const data=JSON.parse(event.data);if(data.id===request){ws.removeEventListener('message',listener);data.error?reject(Error(JSON.stringify(data.error))):resolve(data.result);}};ws.addEventListener('message',listener);ws.send(JSON.stringify({id:request,method,params}));});}
try{
  const endpoint=new URL(await chromeEndpoint(chrome));const pages=await(await fetch('http://'+endpoint.host+'/json/list')).json();ws=new WebSocket(pages.find(p=>p.type==='page').webSocketDebuggerUrl);await socketOpen(ws);
  for(const assembly of ['log-trap','rock-trap']){
    const folder=join(base,assembly+'-native-appearance-v1');await command('Page.navigate',{url:origin+'/@fs/'+join(folder,'index.html')});
    let state;
    for(let i=0;i<150;i++){state=await evaluate(ws,++id,'window.nativeAppearance && ({loaded:window.nativeAppearance.loaded,error:window.nativeAppearance.error})');if(state)break;await new Promise(r=>setTimeout(r,100));}
    if(!state?.loaded)throw Error('Load failed '+JSON.stringify(state));
    await evaluate(ws,++id,"window.nativeAppearance.verify().then(r=>window.__verified=r).catch(e=>window.__verified={status:'FAIL',error:String(e)});true");
    let result;
    for(let i=0;i<300;i++){result=await evaluate(ws,++id,'window.__verified');if(result)break;await new Promise(r=>setTimeout(r,100));}
    const manifest=JSON.parse(await readFile(join(folder,'manifest.json'),'utf8'));result={...result,glb_sha256:createHash('sha256').update(await readFile(join(folder,'native-appearance.glb'))).digest('hex')};
    await writeFile(join(folder,'browser-verification.json'),JSON.stringify(result,null,2)+'\n');if(result.status!=='PASS')throw Error(JSON.stringify(result));
    for(const tick of [0,Math.floor(manifest.terminal_tick/2),manifest.terminal_tick]){const data=await evaluate(ws,++id,`window.nativeAppearance.show(${tick});window.nativeAppearance.capture()`);await writeFile(join(folder,`browser-tick-${tick}.png`),Buffer.from(data.split(',')[1],'base64'));}
    console.log(assembly,result.status,result.samples.length,'phases; terminal clamped');
  }
}finally{ws?.close();chrome.kill('SIGTERM');await closed;await new Promise(r=>server.close(r));await rm(profile,{recursive:true,force:true});}
