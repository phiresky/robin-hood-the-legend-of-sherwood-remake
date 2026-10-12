(async()=>{
 const v=window.reviewViewport,cfg=window.canonicalSmoke,checks=[],missions=[];
 const check=(name,pass,detail)=>{checks.push({name,pass:!!pass,detail});window.canonicalSmokeEvidence={checks,mission:window.currentMission};if(!pass)throw Error(name)};
 const wait=async(fn,label)=>{for(let i=0;i<1800;i++){if(fn())return;const error=document.querySelector('#result')?.textContent;if(error?.startsWith('FAIL'))throw Error(label+': '+error);await new Promise(r=>setTimeout(r,100));}throw Error('Timeout '+label)};
 const select=(label,value)=>{const e=document.querySelector('select[aria-label="'+label+'"]');check('Offered '+label+' '+value,e&&[...e.options].some(o=>o.value===value));e.value=value;e.dispatchEvent(new Event('change',{bubbles:true}));};
 const hash=async data=>[...new Uint8Array(await crypto.subtle.digest('SHA-256',data))].map(n=>n.toString(16).padStart(2,'0')).join('');
 const throws=fn=>{try{fn();return false}catch{return true}};
 const capture=async name=>{window.scatterStage={name,time:Date.now()};await wait(()=>window.scatterCaptures[name],name)};
 const context=async()=>{const snap=v.nativeArt.loopDrawSnapshot(),rows=[];for(const d of snap.draws.filter(d=>!d.element.loop))rows.push({id:d.element.id,source:d.element.source,frame:d.frame,tick:d.sourceTick,active:d.sourceActive,creation:d.element.creation_order,pixels:await hash(d.pixels.data)});return {rows,orderOnly:snap.orderOnly?.filter(e=>!e.loop).map(e=>e.id)};};
 const mode=[...document.querySelectorAll('.editor-modes button')].find(b=>b.textContent.trim()==='Mission');check('Mission UI available',mode);mode.click();await wait(()=>document.querySelector('[aria-label="Mission"]'),'mission chooser');
 window.nativeEffectCrops=[];
 const entries=[...cfg.missions,cfg.missions[0]];
 for(let index=0;index<entries.length;index++){
  const {entry,contract}=entries[index];window.currentMission=entry.mission;
  select('Mission',entry.mission);await wait(()=>v.bindings.document()?.mission?.importedFrom===entry.mission,'mission loaded');
  await wait(()=>document.querySelector('[aria-label="State preview asset"] option[value="'+entry.id+'"]'),'private entry');select('State preview asset',entry.id);
  await wait(()=>v.loopClocks.ready&&v.nativeArt.contract?.mission===entry.mission&&v.missionStates.players.size===1&&v.physicalNativeEffects&&document.querySelector('[aria-label="3D effects over original artwork"]'),'physical binding');
  select('State preview view','art');v.setNativeArtPlaying(false);
  const focus=contract.focus_element_id,loops=contract.native.elements.filter(e=>e.loop),idle=contract.native.elements.filter(e=>!e.loop);
  check('Fifteen independent loops '+entry.mission,loops.length===15&&new Set(loops.map(e=>v.loopClocks.resourceToken(e.id))).size===15);
  v.seekNativeArt(7);check('Global seek affects only loops',v.loopClocks.snapshot().every(r=>r.tick===(loops.some(e=>e.id===r.id)?7:0)));v.seekNativeArt(0);
  const present=v.nativeArt.loopDrawSnapshot();
  for(const state of [...(contract.native.patch_states??[]),...(contract.native.background_states??[])])if(state.initial.length){const draw=present.draws.find(d=>d.element.id===state.id);check('Initial patch artwork retained '+state.id,draw&&draw.sourceTick===0&&draw.element.frames[0].sha256===state.initial[0].sha256);}
  for(const e of idle.filter(e=>!e.active&&e.initial_frame)){const draw=present.draws.find(d=>d.element.id===e.id);check('Explicit idle artwork retained '+e.id,draw&&draw.sourceTick===0&&draw.element.frames[0].sha256===e.initial_frame.sha256);}
  const frozen=JSON.stringify(await context()),initialRows=v.loopClocks.snapshot().filter(r=>idle.some(e=>e.id===r.id));
  for(const e of idle)for(const action of [()=>v.loopClocks.seek(e.id,1),()=>v.loopClocks.setPlaying(e.id,true),()=>v.loopClocks.setActive(e.id,!e.active)])check('Immutable clock rejects '+e.id,throws(action));
  for(let j=0;j<loops.length;j++){const e=loops[j],before=v.loopClocks.snapshot();v.seekNativeArt(j+3,e.id);const after=v.loopClocks.snapshot();check('Independent seek '+e.id,after.every((r,k)=>r.tick===(r.id===e.id?j+3:before[k].tick)));}
  check('Initial nonloop cursors retained',JSON.stringify(v.loopClocks.snapshot().filter(r=>idle.some(e=>e.id===r.id)))===JSON.stringify(initialRows));
  check('Initial draw pixels and ordering retained',JSON.stringify(await context())===frozen,{context:JSON.parse(frozen)});
  for(const e of loops)v.seekNativeArt(0,e.id);
  const input=document.querySelector('[aria-label="3D effects over original artwork"]');if(input.checked)input.click();const original=await hash(v.nativePreviewPixels().data);input.click();
  const physical=await hash(v.nativePreviewPixels().data);check('Physical output differs from original '+entry.mission,physical!==original);
  const player=[...v.missionStates.players.values()][0].player,states=[];
  for(const tick of [0,4,28,52,55,56,0]){v.seekNativeArt(tick,focus);const sha256=await hash(v.nativePreviewPixels().data);check('Exact 56tick phase '+tick,player.tick===tick%56);if(tick===56)check('Exact composed cycle restoration',sha256===physical);states.push({tick,sha256,physicalTick:player.tick});}
  if(index<cfg.missions.length){
   const region=cfg.cropRegion,images=[];
   for(const tick of [0,28])for(const view of ['original','physical']){
    v.seekNativeArt(tick,focus);if(input.checked!==(view==='physical'))input.click();const pixels=v.nativePreviewPixels(),data=new Uint8ClampedArray(region.width*region.height*4),origin=contract.native.origin;
    check('Source registered crop bounds',region.x>=origin[0]&&region.y>=origin[1]&&region.x-origin[0]+region.width<=pixels.width&&region.y-origin[1]+region.height<=pixels.height);
    for(let y=0;y<region.height;y++){const start=((region.y-origin[1]+y)*pixels.width+region.x-origin[0])*4;data.set(pixels.data.subarray(start,start+region.width*4),y*region.width*4);}
    const c=document.createElement('canvas');c.width=region.width;c.height=region.height;c.getContext('2d').putImageData(new ImageData(data,region.width,region.height),0,0);
    const row={name:'tree42-'+index+'-'+view+'-tick'+tick,mission:entry.mission,tick,view,region,origin,sha256:await hash(data),png:c.toDataURL('image/png')};images.push(row);window.nativeEffectCrops.push(row);
   }
   const panel=document.createElement('div');panel.style.cssText='position:fixed;inset:0;background:#222;color:white;z-index:99999;padding:12px;font:14px sans-serif;overflow:auto';const title=document.createElement('p');title.textContent=entry.mission+' — source-camera Tree42 crops; original/physical at ticks0/28; immutable source initial context (startup scripts excluded)';panel.append(title);
   const grid=document.createElement('div');grid.style.cssText='display:flex;gap:8px';panel.append(grid);for(const row of images){const col=document.createElement('div'),label=document.createElement('p'),img=document.createElement('img');label.textContent=row.view+' tick '+row.tick;img.src=row.png;img.style.cssText='width:330px;image-rendering:pixelated';col.append(label,img);grid.append(col);await img.decode();}document.body.append(panel);await capture('tree42-'+index+'-comparison');panel.remove();
  }
  const oldSnapshot=v.nativeArt.loopDrawSnapshot(),token=v.loopClocks.resourceToken(focus);
  missions.push({mission:entry.mission,rebind:index===cfg.missions.length,states,context:JSON.parse(frozen),original,physical});
  select('Mission','');await wait(()=>!v.bindings.document()?.mission?.importedFrom&&!v.loopClocks.ready,'retire mission');
  check('Retired token rejected',!v.loopClocks.resourceIsCurrent(token));check('Retired snapshot rejected',throws(()=>v.nativeArt.composeLoopSnapshot(oldSnapshot)));
  check('Initial context retired',v.nativeArt.initialElements.length===0&&v.nativeArt.initialRanks.size===0&&!v.nativeArt.initialContext&&!v.physicalNativeEffects);await new Promise(r=>setTimeout(r,100));
 }
 return {status:'SCATTER_EXECUTED_PENDING_VISUAL_REVIEW',checks,missions,scope:'Seven real mission UI selections plus first-mission rebind; bounded static receiver, full declared source-initial artwork. Excludes executed startup scripts and full canonical physical scene closure.'};
})()
