import {spawn} from 'node:child_process';
import {mkdtemp, readFile, writeFile, rm} from 'node:fs/promises';
import {resolve, join} from 'node:path';
import {pathToFileURL} from 'node:url';
import {chromeEndpoint, socketOpen, evaluate} from '../../app/tests/cdp.mjs';
const out=resolve(process.argv[2]);
const evidence=JSON.parse(await readFile(join(out,'evidence.json'),'utf8'));
const profile=await mkdtemp('/home/phire/.cache/mixed-review-');
const chrome=spawn('/usr/lib/chromium/chromium',['--headless','--no-sandbox','--disable-gpu','--disable-background-networking','--window-size=1440,1100','--remote-debugging-port=0','--user-data-dir='+profile,pathToFileURL(join(out,'index.html')).href],{stdio:['ignore','ignore','pipe'],env:{...process.env,TMPDIR:'/home/phire/.cache'}});
const closed=new Promise(r=>chrome.on('close',r));let socket,id=0;
try {
 const endpoint=new URL(await chromeEndpoint(chrome,{timeoutMs:30000}));
 const pages=await(await fetch('http://'+endpoint.host+'/json/list')).json();
 socket=new WebSocket(pages.find(p=>p.type==='page').webSocketDebuggerUrl);await socketOpen(socket);
 for(let i=0;i<200;i++) {if(await evaluate(socket,++id,"document.readyState==='complete' && !!document.querySelector('#export')"))break;await new Promise(r=>setTimeout(r,100));}
 const results=await evaluate(socket,++id,`(()=>{
  const fail=m=>{throw Error(m)};
  if(document.querySelectorAll('article').length!==${evidence.card_count})fail('Card count');
  document.querySelectorAll('img').forEach(i=>i.loading='eager');
  document.querySelectorAll('select').forEach(s=>{s.value='approved';s.dispatchEvent(new Event('input',{bubbles:true}))});
  document.querySelector('[data-note="0"]').value='Browser verification only';
  document.querySelector('[data-note="0"]').dispatchEvent(new Event('input',{bubbles:true}));
  document.querySelector('#export').click();
  const text=document.querySelector('#feedback').value;
  const expected=${JSON.stringify(evidence.cards.flatMap(c=>c.members.map(m=>({asset:m.asset_id,scope:m.scope,revision:m.review_revision}))))};
  if(text.split('\\n').length!==expected.length)fail('Paired export count');
  for(const e of expected)if(!text.includes(e.asset+': approved ('+e.scope+')')||!text.includes('[review '+e.revision+']'))fail('Exact scoped revision export');
  return {status:'PASS',cards:${evidence.card_count},decisions:expected.length,exact_scope_export:true};
 })()`);
 let loaded=false;
 for(let i=0;i<300&&!loaded;i++){loaded=await evaluate(socket,++id,"[...document.images].every(i=>i.complete&&i.naturalWidth>0)");if(!loaded)await new Promise(r=>setTimeout(r,100));}
 if(!loaded)throw Error('Images failed to load');results.images=true;
 await evaluate(socket,++id,'location.reload()');await new Promise(r=>setTimeout(r,500));
 results.persistence=await evaluate(socket,++id,"[...document.querySelectorAll('select')].every(s=>s.value==='approved') && document.querySelector('[data-note=\"0\"]').value==='Browser verification only'");
 if(!results.persistence)throw Error('Review persistence failed');
 await evaluate(socket,++id,"localStorage.clear();document.querySelectorAll('select,textarea').forEach(e=>e.value='')");
 const shot=await new Promise((resolve,reject)=>{const request=++id;const timer=setTimeout(()=>reject(Error('Screenshot timeout')),10000);const listener=e=>{const d=JSON.parse(e.data);if(d.id!==request)return;clearTimeout(timer);socket.removeEventListener('message',listener);d.error?reject(d.error):resolve(d.result.data)};socket.addEventListener('message',listener);socket.send(JSON.stringify({id:request,method:'Page.captureScreenshot',params:{format:'png'}}));});
 await writeFile(join(out,'browser.png'),Buffer.from(shot,'base64'));
 await writeFile(join(out,'browser-verification.json'),JSON.stringify(results,null,2)+'\n');console.log(JSON.stringify(results));
} finally {socket?.close();chrome.kill('SIGTERM');await closed;await rm(profile,{recursive:true,force:true});}
