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
  const folder=join(base,process.argv[2]||'scoped-geometry-review-v2','gallery');
  await command('Emulation.setDeviceMetricsOverride',{width:1440,height:1100,deviceScaleFactor:1,mobile:false});
  await command('Page.navigate',{url:origin+'/@fs/'+join(folder,'index.html')});
  for(let i=0;i<150;i++){if(await evaluate(ws,++id,'document.querySelectorAll("article").length===4'))break;await new Promise(r=>setTimeout(r,100));}
  await evaluate(ws,++id,'document.querySelectorAll("img").forEach(i=>i.loading="eager");true');
  let result;
  for(let i=0;i<150;i++){result=await evaluate(ws,++id,'({cards:[...document.querySelectorAll("article")].map(a=>({id:a.id,revision:a.dataset.reviewRevision,approveEnabled:!a.querySelector("option[value=approved]").disabled})),images:[...document.images].map(i=>({src:i.getAttribute("src"),loaded:i.complete&&i.naturalWidth>0}))})');if(result.images.every(i=>i.loaded))break;await new Promise(r=>setTimeout(r,100));}
  if(result.cards.length!==4||result.cards.some(c=>!c.approveEnabled)||result.images.some(i=>!i.loaded))throw Error(JSON.stringify(result));
  for(const card of result.cards){await evaluate(ws,++id,`document.getElementById(${JSON.stringify(card.id)}).scrollIntoView();true`);const shot=await command('Page.captureScreenshot',{format:'png'});await writeFile(join(folder,card.id+'-browser.png'),Buffer.from(shot.data,'base64'));}
  result.status='PASS gallery load and evidence image availability';result.evidence_sha256=createHash('sha256').update(await readFile(join(folder,'evidence.json'))).digest('hex');await writeFile(join(folder,'browser-verification.json'),JSON.stringify(result,null,2)+'\n');console.log(result.status,result.cards.length,result.images.length);

}finally{ws?.close();chrome.kill('SIGTERM');await closed;await new Promise(r=>server.close(r));await rm(profile,{recursive:true,force:true});}
