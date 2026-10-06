// Bounded state checks inside an already loaded, matching refined editor fixture.
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import {createHash} from 'node:crypto';
const base=resolve('level-editor/work/croisement02-refinement/restart2-state');
const stage=join(base,'remaining-seven-package-v2');
const json=async p=>JSON.parse(await readFile(p,'utf8'));
const sha=b=>createHash('sha256').update(b).digest('hex');
export async function run({ws,nextId,evaluate,out}){
 await mkdir(out,{recursive:true});
 const manifest=await json(join(stage,'manifest.json')),checks=[],screenshots=[];
 const command=(method,params)=>new Promise((resolve,reject)=>{const id=nextId();const listener=e=>{const d=JSON.parse(e.data);if(d.id===id){ws.removeEventListener('message',listener);d.error?reject(Error(JSON.stringify(d.error))):resolve(d.result)}};ws.addEventListener('message',listener);ws.send(JSON.stringify({id,method,params}));});
 // Use this module's CDP evaluator so the callback's return conventions cannot alter assertions.
 const exec=async expression=>{const r=await command('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true});if(r.exceptionDetails)throw Error(JSON.stringify(r.exceptionDetails));return r.result?.value};
 const until=async expression=>{for(let i=0;i<600;i++){if(await exec(expression))return;await new Promise(r=>setTimeout(r,200));}throw Error('Timeout '+expression+' '+await exec('document.body.innerText.slice(-1600)'))};
 const check=async(name,expression)=>{const pass=await exec(expression);checks.push({name,pass});if(!pass)throw Error(name)};
 const shot=async name=>{await exec('new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))');const r=await command('Page.captureScreenshot',{format:'png'});await writeFile(join(out,name+'.png'),Buffer.from(r.data,'base64'));screenshots.push(name+'.png')};
 const select=async(label,value)=>exec(`(()=>{const s=document.querySelector('select[aria-label="${label}"]');if(!s)throw Error('Missing ${label}');s.value=${JSON.stringify(value)};s.dispatchEvent(new Event('change',{bubbles:true}))})()`);
 const nativeHash=`(async()=>Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',window.reviewViewport.stateDelivery.native.pixels().data)),v=>v.toString(16).padStart(2,'0')).join(''))()`;
 const cart=await json(join(base,'cart-contract-preparation-v2/independent-reference.json')),fence=await json(join(base,'fence-contract-preparation-v2/independent-reference.json'));
 try{
 await exec(`(()=>{if(!document.querySelector('select[aria-label="Mission"]'))[...document.querySelectorAll('button')].find(b=>b.textContent==='Mission').click()})()`);
 await until(`!!document.querySelector('select[aria-label="Mission"]')`);
 let mission='';
 for(const entry of manifest.entries){
  const contract=await json(join(stage,'library',entry.contract.path)),family=contract.families[0],fid=JSON.stringify(family.id);
  if(mission!==entry.mission){await select('Mission',entry.mission);mission=entry.mission;}
  await until(`!!document.querySelector('[aria-label="State preview asset"] option[value="${entry.id}"]')`);
  await select('State preview asset',entry.id);
  await until(`window.reviewViewport.stateDelivery.ready&&window.reviewViewport.stateDelivery.contract.native.mission===${JSON.stringify(entry.mission)}&&window.reviewViewport.stateDelivery.contract.families[0].id===${fid}&&!document.querySelector('[aria-label="State preview"] [role="status"]')`);
  await check(entry.id+' exact contract',`JSON.stringify(window.reviewViewport.stateDelivery.contract)===${JSON.stringify(JSON.stringify(contract))}`);
  await check(entry.id+' native initially paused',`window.reviewViewport.statePresentationMode==='native-art'&&!window.reviewViewport.stateDelivery.native.isPlaying`);
  const isFence=entry.id.endsWith('south-field-fence'),gold=(isFence?fence:cart).records.find(r=>r.contract.endsWith(entry.contract.path.split('/').at(-1)));
  if(gold)await check(entry.id+' initial source pixels',`${nativeHash}.then(h=>h===${JSON.stringify(gold.cases[0].rgba_sha256)})`);
  for(const tick of [...new Set([0,Math.floor(family.body_terminal_tick/2),family.body_terminal_tick])]){
   await exec(`window.reviewViewport.stateDelivery.native.seek(${isFence?0:tick});window.reviewViewport.seekDeliveredState(${fid},${tick})`);
   await check(entry.id+' seek '+tick,`window.reviewViewport.deliveredStateStatus(${fid}).tick===${tick}`);
   if(gold){const expected=gold.cases.find(c=>c.tick===tick&&(!isFence||c.phase==='forward'));await check(entry.id+' source pixels '+tick,`${nativeHash}.then(h=>h===${JSON.stringify(expected.rgba_sha256)})`);}
  }
  await exec(`(()=>{const input=document.querySelector('[aria-label="State preview frame"]');input.value='${family.body_terminal_tick}';input.dispatchEvent(new Event('input',{bubbles:true}))})()`);
  await check(entry.id+' actual frame control',`window.reviewViewport.deliveredStateStatus(${fid}).tick===${family.body_terminal_tick}`);
  await shot(entry.id+'-native');
  for(const endpoint of ['initial','applied']){
   await select('State preview view',endpoint);
   const count=Array.isArray(family.physical[endpoint])?family.physical[endpoint].length:0;
   await check(entry.id+' physical '+endpoint,`(()=>{const d=window.reviewViewport.stateDelivery,p=d.roots.get(${fid});return d.mode==='physical-endpoint'&&d.physical.visible&&p.${endpoint}.visible&&!p.${endpoint==='initial'?'applied':'initial'}.visible&&p.${endpoint}.children.length===${count}})()`);
   if(isFence)await check(entry.id+' suppresses only exact fence parts '+endpoint,`(()=>{const v=window.reviewViewport,ids=['building-019','building-020'];return ids.every(id=>v.partViews.has(id)&&!v.partViews.get(id).wrapper.visible)&&[...v.partViews].filter(([id,p])=>!ids.includes(id)).every(([id,p])=>p.wrapper.visible===!v.bindings.document().objects.find(o=>o.id===id).hidden)})()`);
   await shot(entry.id+'-'+endpoint);
  }
  await select('State preview view','art');
  if(isFence)await check(entry.id+' restores original static fence',`['building-019','building-020'].every(id=>window.reviewViewport.partViews.get(id).wrapper.visible)`);
  await exec(`window.reviewViewport.stateDelivery.native.seek(0);window.reviewViewport.resetDeliveredState(${fid})`);
  if(gold)await check(entry.id+' reset source pixels',`${nativeHash}.then(h=>h===${JSON.stringify(gold.cases[0].rgba_sha256)})`);
  await check(entry.id+' reset restores native mode',`!window.reviewViewport.stateDelivery.physical.visible&&window.reviewViewport.deliveredStateStatus(${fid}).tick===undefined`);
 }
 await select('Mission','');await until(`!document.querySelector('[aria-label="State preview view"]')`);
 await check('Map-only removes all state roots',`!window.reviewViewport.stateDelivery.ready&&window.reviewViewport.stateDelivery.physical.children.length===0&&window.reviewViewport.statePresentationMode==='physical'`);
 const index=await readFile('level-editor/library/mission-states/index.json');if(sha(index)!==manifest.installed_index_sha256)throw Error('Installed index changed');
 const evidence=[];for(const name of screenshots)evidence.push({path:name,sha256:sha(await readFile(join(out,name)))});
 await writeFile(join(out,'verification.json'),JSON.stringify({status:'PASS',manifest_sha256:sha(await readFile(join(stage,'manifest.json'))),checks,evidence,installed_index_sha256:sha(index),scope:'Seven private controlled previews on the matching refined static fixture; no live catalog publication, actor simulation or inferred physical transition.'},null,2)+'\n');
 return {status:'PASS',checks:checks.length,out};
 }catch(error){await shot('failure').catch(()=>{});await writeFile(join(out,'failure.json'),JSON.stringify({error:String(error),checks},null,2)+'\n');throw error;}
}
