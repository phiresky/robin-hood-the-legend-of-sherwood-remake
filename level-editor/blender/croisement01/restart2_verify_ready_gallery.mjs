import {spawn} from 'node:child_process';
import {mkdtemp, readFile, writeFile, rm} from 'node:fs/promises';
import {resolve, join} from 'node:path';
import {pathToFileURL} from 'node:url';
import {chromeEndpoint, socketOpen, evaluate} from '../../app/tests/cdp.mjs';

const out=resolve(process.argv[2]??'level-editor/work/croisement01-refinement/restart2/ready-trees18-20-v2/gallery');
const artifactStem=process.argv[3]??'ready-trees18-20';
if(!/^[a-zA-Z0-9_-]+$/.test(artifactStem))throw Error('Unsafe artifact stem');
const evidence=JSON.parse(await readFile(join(out,'evidence.json'),'utf8'));
const target=evidence.items.find(item=>item.id==='croisement01-tree-18')??evidence.items[0];
if(!target)throw Error('No pending asset to exercise review controls');
const profile=await mkdtemp('/home/phire/.cache/croisement01-ready-gallery-');
const url=pathToFileURL(join(out,'index.html')).href;
const chrome=spawn('/usr/lib/chromium/chromium',['--headless','--no-sandbox','--disable-gpu',
  '--disable-background-networking','--window-size=1440,1100','--remote-debugging-port=0',
  '--user-data-dir='+profile,url],{stdio:['ignore','ignore','pipe'],env:{...process.env,TMPDIR:'/home/phire/.cache'}});
const closed=new Promise(resolve=>chrome.on('close',resolve));let socket,id=0;
try {
  const endpoint=new URL(await chromeEndpoint(chrome,{timeoutMs:30000}));
  const pages=await (await fetch('http://'+endpoint.host+'/json/list')).json();
  socket=new WebSocket(pages.find(p=>p.type==='page').webSocketDebuggerUrl);await socketOpen(socket);
  async function ready(){
    for(let i=0;i<200;i++){
      if(await evaluate(socket,++id,"document.readyState==='complete' && !!document.querySelector('#copy-reviews')"))return;
      await new Promise(resolve=>setTimeout(resolve,100));
    }
    throw Error('Gallery did not load');
  }
  await ready();
  const checks=await evaluate(socket,++id,`(() => {
    const assert=(v,m)=>{if(!v)throw Error(m)};
    assert(document.querySelectorAll('article').length===${evidence.items.length},'Card count');
    const card=document.querySelector('#${target.id}');
    card.querySelector('.decision').value='approved';card.querySelector('.decision').dispatchEvent(new Event('change',{bubbles:true}));
    assert(card.querySelector('.decision').value==='approved','Approve button');

    card.querySelector('.decision').value='needs refinement';card.querySelector('.decision').dispatchEvent(new Event('change',{bubbles:true}));
    const note=card.querySelector('.review-note');note.value='Automated browser test';note.dispatchEvent(new Event('input',{bubbles:true}));
    assert(document.querySelector('#review-export').value.includes(card.id+': needs refinement — Automated browser test'),'Request changes export');
    assert(!document.querySelector('#copy-reviews').disabled,'Copy enabled');
    return {status:'PASS',cards:${evidence.items.length},decision_select:true,feedback_export:true,search:false};
  })()`);
  await evaluate(socket,++id,'location.reload();');
  await new Promise(resolve=>setTimeout(resolve,500));await ready();
  const restored=await evaluate(socket,++id,`(() => {
    const card=document.querySelector('#${target.id}');
    if(card.querySelector('.decision').value!=='needs refinement'||card.querySelector('.review-note').value!=='Automated browser test')throw Error('Review persistence failed');
    document.querySelector('#clear-reviews').click();
    if(document.querySelector('#review-export').value)throw Error('Clear failed');
    card.scrollIntoView();return true;
  })()`);
  checks.persistence=restored;checks.clear=true;
  await evaluate(socket,++id,`(() => {
    const images=[...document.querySelectorAll('#${target.id} img')];
    if(images.length!==6)throw Error('Expected six review images');
    for(const image of images)image.loading='eager';
  })()`);
  checks.images=false;
  for(let i=0;i<200&&!checks.images;i++){
    checks.images=await evaluate(socket,++id,`[...document.querySelectorAll('#${target.id} img')].every(image=>image.complete&&image.naturalWidth>0)`);
    if(!checks.images)await new Promise(resolve=>setTimeout(resolve,100));
  }
  if(!checks.images)throw Error('Review images did not load');
  const screenshot=await new Promise((resolve,reject)=>{
    const request=++id;const timer=setTimeout(()=>reject(Error('Screenshot timeout')),10000);
    const listener=event=>{const data=JSON.parse(event.data);if(data.id!==request)return;clearTimeout(timer);socket.removeEventListener('message',listener);data.error?reject(data.error):resolve(data.result.data)};
    socket.addEventListener('message',listener);socket.send(JSON.stringify({id:request,method:'Page.captureScreenshot',params:{format:'png'}}));
  });
  await writeFile(join(out,'../../'+artifactStem+'-browser.png'),Buffer.from(screenshot,'base64'));
  await writeFile(join(out,'../../'+artifactStem+'-browser-verification.json'),JSON.stringify(checks,null,2)+'\n');
  console.log(JSON.stringify(checks));
}finally{
  socket?.close();chrome.kill('SIGTERM');await closed;await rm(profile,{recursive:true,force:true});
}
