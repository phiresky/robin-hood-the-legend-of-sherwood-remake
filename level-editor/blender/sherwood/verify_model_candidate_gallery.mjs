import {spawn} from 'node:child_process';
import {mkdir,mkdtemp,writeFile,readFile} from 'node:fs/promises';
import {chromeEndpoint,socketOpen} from '../../app/tests/cdp.mjs';
const expectedIds=JSON.parse(await readFile('work/needs-review/sherwood-refinement/northeast-oak-review/gallery-manifest.json','utf8')).items.map(i=>i.id);
const root='/home/phire/.cache/sccache/sherwood-gallery-browser';await mkdir(root,{recursive:true});
const profile=await mkdtemp(root+'/p-');
const chrome=spawn('/usr/lib/chromium/chromium',['--headless','--no-sandbox','--disable-dev-shm-usage','--disable-background-networking','--window-size=1440,1100','--remote-debugging-port=0','--user-data-dir='+profile,'about:blank'],{stdio:['ignore','ignore','pipe'],env:{...process.env,TMPDIR:root}});
const closed=new Promise(r=>chrome.once('close',r));let ws,id=0;
try{
 const endpoint=new URL(await chromeEndpoint(chrome));
 const pages=await(await fetch('http://'+endpoint.host+'/json/list')).json();
 ws=new WebSocket(pages.find(p=>p.type==='page').webSocketDebuggerUrl);await socketOpen(ws);
 const call=(method,params={})=>new Promise((resolve,reject)=>{
  const n=++id;const timer=setTimeout(()=>{ws.removeEventListener('message',receive);reject(Error('CDP timeout'));},30000);
  function receive(e){const d=JSON.parse(e.data);if(d.id!==n)return;clearTimeout(timer);ws.removeEventListener('message',receive);d.error?reject(d.error):resolve(d.result)}
  ws.addEventListener('message',receive);ws.send(JSON.stringify({id:n,method,params}));
 });
 const evaluate=async expression=>{const r=await call('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true});if(r.exceptionDetails)throw Error(JSON.stringify(r.exceptionDetails));return r.result.value};
 await call('Page.enable');
 await call('Page.navigate',{url:'http://127.0.0.1:5190/revisions/'});
 for(let attempt=0;attempt<100;attempt++){
  try{if(await evaluate("!!document.querySelector('article')"))break;}catch{}
  await new Promise(r=>setTimeout(r,100));
 }
 const result=await evaluate(`(async()=>{for(let i=0;i<100&&!document.querySelector('article');i++)await new Promise(r=>setTimeout(r,100));
 const cards=[...document.querySelectorAll('article')];const expectedIds=${JSON.stringify(expectedIds)};if(JSON.stringify(cards.map(c=>c.id))!==JSON.stringify(expectedIds))throw Error('Unexpected pending cards');const card=cards[0];if(!card)throw Error('No pending card');
 card.scrollIntoView();for(const img of card.querySelectorAll('img')){img.loading='eager';await img.decode();}
 const ready=cards.filter(c=>[...c.querySelector('.decision').options].some(o=>o.value==='approved'&&!o.disabled)).length;
 if(ready!==expectedIds.length || cards.length!==expectedIds.length)throw Error('Missing enabled approvals: '+ready);
 const note=card.querySelector('.review-note');note.value='Gallery verification note';note.dispatchEvent(new Event('input',{bubbles:true}));
 if(!document.querySelector('#review-export').value.includes(card.id+': feedback — Gallery verification note'))throw Error('Feedback export failed');
 return {status:'PASS',cards:cards.length,ready,images_loaded:[...card.querySelectorAll('img')].every(i=>i.naturalWidth>0),feedback_export:true};})()`);
 const shot=await call('Page.captureScreenshot',{format:'png'});await writeFile('work/needs-review/sherwood-refinement/northeast-oak-review/browser.png',Buffer.from(shot.data,'base64'));
 await evaluate(`document.querySelector('#clear-reviews').click()`);
 await writeFile('work/needs-review/sherwood-refinement/northeast-oak-review/browser-check.json',JSON.stringify(result,null,2)+'\n');console.log(JSON.stringify(result));
}finally{ws?.close();chrome.kill('SIGTERM');await closed;}
