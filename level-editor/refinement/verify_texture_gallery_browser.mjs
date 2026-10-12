/** Verify real texture-gallery feedback in a disposable Chromium profile. */
import {spawn} from 'node:child_process';
import {readFile,writeFile,mkdir,mkdtemp,rm} from 'node:fs/promises';
import {resolve,join,dirname} from 'node:path';
import {fileURLToPath} from 'node:url';
import crypto from 'node:crypto';
import {chromeEndpoint,socketOpen} from '../app/tests/cdp.mjs';

const gallery=resolve(process.argv[2]??'level-editor/work/needs-review/nottingham-refinement/texture-review/gallery');
const output=resolve(process.argv[3]??join(gallery,'../browser-verification'));
const origin=process.env.REVIEW_ORIGIN??'http://localhost:5180';
const url=origin+'/@fs'+gallery+'/index.html';
await mkdir(output,{recursive:true});
const profile=await mkdtemp(join(output,'isolated-profile-'));
const digest=bytes=>crypto.createHash('sha256').update(bytes).digest('hex');
const evidence=JSON.parse(await readFile(join(gallery,'evidence.json'),'utf8'));
const expectedIds=evidence.items.map(item=>item.id);
const decisionsPath=join(dirname(gallery),'decisions.json');
const decisionsBefore=await readFile(decisionsPath).catch(e=>{if(e.code==='ENOENT')return null;throw e;});
const chrome=spawn('/usr/lib/chromium/chromium',['--headless','--no-sandbox','--disable-dev-shm-usage',
  '--disable-background-networking','--window-size=1400,1100','--remote-debugging-port=0','--user-data-dir='+profile,url],
  {stdio:['ignore','ignore','pipe'],env:{...process.env,TMPDIR:'/tmp'}});
let browserLog='';
chrome.stderr.on('data',chunk=>{browserLog+=chunk.toString();});
const closed=new Promise(r=>chrome.on('close',r));
let ws,id=0,result={status:'FAIL',gallery,url,isolated_profile:true};
async function command(method,params={}){
 const request=++id;
 return new Promise((resolve,reject)=>{
  const timer=setTimeout(()=>{ws.removeEventListener('message',listener);reject(new Error(method+' timeout'));},30000);
  const listener=event=>{const response=JSON.parse(event.data);if(response.id!==request)return;
    clearTimeout(timer);ws.removeEventListener('message',listener);response.error?reject(new Error(JSON.stringify(response.error))):resolve(response.result);};
  ws.addEventListener('message',listener);ws.send(JSON.stringify({id:request,method,params}));
 });
}
async function inspect(expression){const value=await command('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true});if(value.exceptionDetails)throw new Error(JSON.stringify(value.exceptionDetails));return value.result?.value;}
async function capture(name){const shot=await command('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});await writeFile(join(output,name+'.png'),Buffer.from(shot.data,'base64'));}
try{
 const endpoint=new URL(await chromeEndpoint(chrome,{timeoutMs:30000}));
 const pages=await(await fetch('http://'+endpoint.host+'/json/list')).json();
 ws=new WebSocket(pages.find(p=>p.type==='page').webSocketDebuggerUrl);await socketOpen(ws);
 await command('Page.enable');
 for(let i=0;i<120;i++){
  if(await inspect("document.readyState==='complete' && !!document.querySelector('#review-export')"))break;
  if(i===119)throw new Error('Gallery did not load');await new Promise(r=>setTimeout(r,250));
 }
 const loaded=await inspect(`(async()=>{
   const cards=[...document.querySelectorAll('article[data-review-revision]')];
   const images=[...document.images];images.forEach(i=>i.loading='eager');
   await Promise.all(images.map(i=>i.decode().catch(()=>{})));
   return {ids:cards.map(c=>c.id),revisions:cards.map(c=>[c.id,c.dataset.reviewRevision]),
     images:images.length,broken:images.filter(i=>!i.complete||!i.naturalWidth).map(i=>i.src),
     stateFigures:document.querySelectorAll('figure[data-kind^="texture_state_"]').length,
     stateSections:document.querySelectorAll('.animation-state').length,title:document.title,
     freshExport:document.querySelector('#review-export').value};
 })()`);
 if(JSON.stringify(loaded.ids)!==JSON.stringify(expectedIds))throw new Error('Rendered asset IDs differ from gallery evidence: '+JSON.stringify({actual:loaded.ids,expected:expectedIds,title:loaded.title}));
 if(loaded.broken.length)throw new Error('Broken gallery images: '+loaded.broken.join(','));
 if(!loaded.ids.length)throw new Error('Need an actual review card');
 if(loaded.freshExport)throw new Error('Disposable browser unexpectedly contains saved reviews');
 await capture('gallery-initial');
 const exercised=await inspect(`(()=>{
   const cards=[...document.querySelectorAll('article[data-review-revision]')],a=cards[0],b=cards[1]??cards[0],single=cards.length===1;
   const select=(element,value)=>{element.value=value;element.dispatchEvent(new Event('change',{bubbles:true}));};
   const note=(card,text)=>{const element=card.querySelector('.review-note');element.value=text;element.dispatchEvent(new Event('input',{bubbles:true}));};
   const aDecision=a.querySelector('.decision').querySelector('option[value="approved"]')?'approved':'needs refinement';
   select(a.querySelector('.decision'),aDecision);note(a,'QA ONLY asset A');
   if(single)select(a.querySelector('.decision'),'');
   // Reproduce the historical browser-restoration bug: a foreign select value
   // appears without a user change event while the user edits this card's note.
   b.querySelector('.decision').value='approved';note(b,'QA ONLY asset B');
   const intermediate=document.querySelector('#review-export').value;
   if(!intermediate.includes(b.id+': feedback — QA ONLY asset B'))throw Error('Native restoration contaminated another card');
   select(b.querySelector('.decision'),'needs refinement');
   const expected=document.querySelector('#review-export').value;
   const status=b.querySelector('.status'),oldStatus=status.textContent;status.textContent='fix-needed';
   select(document.querySelector('#readiness'),'ready');if(!b.hidden)throw Error('Pending filter did not hide simulated held card');
   if(document.querySelector('#review-export').value!==expected)throw Error('Filtering changed review ownership');
   select(document.querySelector('#readiness'),'all');status.textContent=oldStatus;
   for(const mode of ['solid','textured','both']){
     select(document.querySelector('#mode'),mode);
     if(document.body.dataset.mode!==mode)throw Error('Mode switch failed');
     if(document.querySelector('#review-export').value!==expected)throw Error('View mode changed review ownership');
   }
   for(const detail of document.querySelectorAll('.animation-state')){detail.open=true;detail.open=false;}
   if(!single){b.after(a);if(b.nextElementSibling!==a)throw Error('Card reorder failed');}document.dispatchEvent(new Event('visibilitychange'));window.dispatchEvent(new PageTransitionEvent('pageshow'));
   if(document.querySelector('#review-export').value!==expected)throw Error('Card reorder/restoration changed exported IDs');
   Object.defineProperty(navigator,'clipboard',{configurable:true,value:{writeText:async text=>{window.__capturedReviewClipboard=text;}}});
   document.querySelector('#copy-reviews').click();
   return {assetA:a.id,assetB:b.id,aDecision,expected,intermediate,crossCardAndReorderTested:!single,clipboardIntercepted:true,simulatedHeldFilter:true};
 })()`);
 await new Promise(r=>setTimeout(r,50));
 if(await inspect('window.__capturedReviewClipboard')!==exercised.expected)throw new Error('Clipboard handler exported the wrong cards');
 await capture('feedback-export');
 await command('Page.reload',{ignoreCache:true});
 for(let i=0;i<120;i++){
  if(await inspect("document.readyState==='complete' && document.querySelector('#review-export')?.value.includes('QA ONLY asset B')"))break;
  if(i===119)throw new Error('Revision-keyed feedback did not restore after reload');await new Promise(r=>setTimeout(r,250));
 }
 const restored=await inspect(`(()=>{
   const a=document.getElementById(${JSON.stringify(exercised.assetA)}),b=document.getElementById(${JSON.stringify(exercised.assetB)});
   return {export:document.querySelector('#review-export').value,
     a:[a.querySelector('.decision').value,a.querySelector('.review-note').value],
     b:[b.querySelector('.decision').value,b.querySelector('.review-note').value],
     storage:Object.keys(localStorage).filter(k=>k.startsWith('model-review-v1:')).map(k=>JSON.parse(localStorage.getItem(k)))};
 })()`);
 if(restored.export!==exercised.expected || JSON.stringify(restored.a)!==JSON.stringify(exercised.crossCardAndReorderTested?[exercised.aDecision,'QA ONLY asset A']:['needs refinement','QA ONLY asset B']) ||
    JSON.stringify(restored.b)!==JSON.stringify(['needs refinement','QA ONLY asset B']))throw new Error('Reload feedback mismatch: '+JSON.stringify({expected:exercised,restored}));
 const revisions=new Map(loaded.revisions);
 if(restored.storage.some(r=>revisions.get(r.asset_id)!==r.revision))throw new Error('Saved draft lost exact asset/revision identity');
 await inspect("document.querySelector('#clear-reviews').click()");
 if(await inspect("document.querySelector('#review-export').value")!=='')throw new Error('Clear reviews failed');
 const decisionsAfter=await readFile(decisionsPath).catch(e=>{if(e.code==='ENOENT')return null;throw e;});
 if((decisionsBefore&&digest(decisionsBefore))!==(decisionsAfter&&digest(decisionsAfter)))throw new Error('Browser test changed user decision file');
 result={status:'PASS',gallery,url,isolated_profile:true,user_decisions_unchanged:true,
   gallery_html_sha256:digest(await readFile(join(gallery,'index.html'))),
   evidence_sha256:digest(await readFile(join(gallery,'evidence.json'))),...loaded,
   feedback:{...exercised,restored},screenshots:['gallery-initial.png','feedback-export.png']};
}catch(error){result.error=String(error);result.browser_log=browserLog;process.exitCode=1;}
finally{
 await writeFile(join(output,'report.json'),JSON.stringify(result,null,2)+'\n');
 console.log(JSON.stringify({status:result.status,cards:result.ids?.length,images:result.images,error:result.error,report:join(output,'report.json')}));
 ws?.close();chrome.kill('SIGTERM');await closed;await rm(profile,{recursive:true,force:true});
}
