(async()=>{
 const v=window.reviewViewport,cfg=window.canonicalSmoke,checks=[];
 const check=(name,pass,detail)=>{checks.push({name,pass:!!pass,detail});window.canonicalSmokeEvidence={checks};if(!pass)throw Error(name)};
 const wait=async(fn,label)=>{for(let i=0;i<1200;i++){if(fn())return;await new Promise(r=>setTimeout(r,100))}throw Error('Timeout '+label)};
 const select=(label,value)=>{const el=document.querySelector('select[aria-label="'+label+'"]');check('Offered '+label,el&&[...el.options].some(o=>o.value===value));el.value=value;el.dispatchEvent(new Event('change',{bubbles:true}))};
 const digest=async p=>[...new Uint8Array(await crypto.subtle.digest('SHA-256',p.data))].map(n=>n.toString(16).padStart(2,'0')).join('');
 const capture=async name=>{window.scatterStage={name,time:Date.now()};await wait(()=>window.scatterCaptures[name],name+' capture')};
 const mode=[...document.querySelectorAll('.editor-modes button')].find(b=>b.textContent.trim()==='Mission');check('Mission mode',mode);mode.click();
 await wait(()=>document.querySelector('select[aria-label="Mission"]'),'mission chooser');
 select('Mission',cfg.entry.mission);await wait(()=>v.bindings.document()?.mission?.importedFrom===cfg.entry.mission,'mission load');
 await wait(()=>document.querySelector('[aria-label="State preview asset"] option[value="'+cfg.entry.id+'"]'),'private state entry');
 select('State preview asset',cfg.entry.id);await wait(()=>v.loopClocks.ready&&v.missionStates.players.size===1&&document.querySelector('[aria-label="3D effects over original artwork"]'),'bound physical effect');
 select('State preview view','art');
 const checkbox=document.querySelector('[aria-label="3D effects over original artwork"]');
 const focus=cfg.focus;
 v.setNativeArtPlaying(false);v.seekNativeArt(0,focus);
 const art=await digest(v.nativePreviewPixels());
 checkbox.click();check('Actual checkbox enabled',checkbox.checked&&v.physicalNativeEffectsEnabled);
 const initial=await digest(v.nativePreviewPixels());check('Saved geometry replaces source sprite',initial!==art,{art,physical:initial});
 await capture('b05-physical-over-art-tick0');
 const player=[...v.missionStates.players.values()][0].player;
 const states=[];
 for(const tick of [0,1,2,72,73,74,98,99,100,196,197,198]){
   v.seekNativeArt(tick,focus);
   const p=v.nativePreviewPixels();states.push({tick,physical_tick:player.tick,sha256:await digest(p)});
   check('Exact sampled tick '+tick,player.tick===tick%198);
 }
 check('Loop restores initial composed bytes',states.at(-1).sha256===initial);
 v.seekNativeArt(74,focus);await capture('b05-physical-over-art-tick74');
 v.seekNativeArt(0,focus);checkbox.click();check('Checkbox disables substitution',!v.physicalNativeEffectsEnabled&&await digest(v.nativePreviewPixels())===art);
 await capture('b05-original-art-tick0');
 select('State preview view','initial');await wait(()=>v.statePresentationMode==='physical','ordinary physical mode');
 check('Ordinary physical orbit remains available',!!v.orbit.enabled&&!v.nativeSurface);
 select('State preview view','art');
 select('Mission','');await wait(()=>!v.bindings.document()?.mission?.importedFrom,'mission retired');
 check('Physical substitution retires with mission',!v.physicalNativeEffects&&!v.physicalNativeEffectsEnabled);
 return {status:'SCATTER_EXECUTED_PENDING_VISUAL_REVIEW',checks,states,scope:'Real UI and source-bound saved B05 geometry on native artwork; no geometry approval, actor crossing, or refined-terrain closure.'};
})()
