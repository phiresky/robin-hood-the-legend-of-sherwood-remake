// Reuse an existing isolated browser after the full Editor suite finishes.
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import {createHash} from 'node:crypto';
import {stateProofCommand} from './restart6_verify_remaining_state_editor.mjs';
export async function run({ws,nextId,origin,out}){
 const base=resolve('level-editor/work/croisement02-refinement/restart2-state/remaining-seven-contact-v1');
 await mkdir(out,{recursive:true});
 const command=(method,params)=>stateProofCommand(ws,nextId,method,params);
 const evaluate=async expression=>{const r=await command('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true});if(r.exceptionDetails)throw Error(JSON.stringify(r.exceptionDetails));return r.result?.value};
 // Navigation intentionally replaces the previous document before any proof evaluation starts.
 const id=nextId();await new Promise((resolve,reject)=>{const timer=setTimeout(()=>{ws.removeEventListener('message',listener);reject(Error('Navigation timed out'))},60000);const listener=e=>{const d=JSON.parse(e.data);if(d.id===id){clearTimeout(timer);ws.removeEventListener('message',listener);d.error?reject(Error(JSON.stringify(d.error))):resolve(d.result)}};ws.addEventListener('message',listener);ws.send(JSON.stringify({id,method:'Page.navigate',params:{url:origin+'/@fs'+join(base,'index.html')}}))});
 let ready=false;for(let i=0;i<1200;i++){try{ready=await evaluate('!!window.stateContactProof?.ready')}catch(e){if(i>5)throw e;}if(ready)break;await new Promise(r=>setTimeout(r,100))}
 if(!ready)throw Error('Contact fixture did not become ready');
 const checks=await evaluate('stateContactProof.checks'),views=[];
 for(let index=0;index<4;index++)for(const endpoint of ['initial','applied'])for(const view of [0,1]){
  const details=await evaluate(`stateContactProof.show(${index},${JSON.stringify(endpoint)},${view})`),png=await evaluate('stateContactProof.capture()'),bytes=Buffer.from(png.split(',')[1],'base64'),name=`${index}-${endpoint}-${view}.png`;
  await writeFile(join(out,name),bytes);views.push({...details,image:name,sha256:createHash('sha256').update(bytes).digest('hex')});
 }
 const receipt={status:'RENDERED_FOR_VISUAL_REVIEW',checks,views,manifest_sha256:createHash('sha256').update(await readFile(join(base,'manifest.json'))).digest('hex'),scope:'Four distinct physical families on exact ground/bank receivers, original camera direction first. Foliage/actors intentionally absent. Same assets; no second full-map load.'};
 await writeFile(join(out,'verification.json'),JSON.stringify(receipt,null,2)+'\n');return receipt;
}
