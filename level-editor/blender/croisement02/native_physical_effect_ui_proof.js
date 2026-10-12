(async()=>{
 const v=window.reviewViewport,cfg=window.canonicalSmoke,checks=[];
 const check=(name,pass,detail)=>{checks.push({name,pass:!!pass,detail});window.canonicalSmokeEvidence={checks};if(!pass)throw Error(name)};
 if(cfg.boundedReceiver){const doc=v.bindings.document();check('Bounded receiver subset is explicit',doc.assetSources.length===cfg.receiverAssets.length&&doc.assetSources.every(s=>cfg.receiverAssets.includes(s.id)),{included:doc.assetSources.map(s=>s.id),scope:'Not a full-map proof'});}
 const wait=async(fn,label)=>{for(let i=0;i<1200;i++){if(fn())return;const failure=document.querySelector('#result')?.textContent;if(failure?.startsWith('FAIL'))throw Error(label+': '+failure);await new Promise(r=>setTimeout(r,100))}throw Error('Timeout '+label)};
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
 if(cfg.refined){
   const receiver=document.querySelector('[aria-label="Refined map background"]');check('Refined receiver UI offered',receiver);
   v.scene.updateMatrixWorld(true);
   const sceneState=()=>{const rows=[];v.scene.traverse(o=>rows.push([o.uuid,o.visible,o.matrixWorld.elements]));return JSON.stringify(rows)};
   const beforeState=sceneState(),beforeImage=await digest(v.nativePreviewPixels());
   receiver.click();check('Refined receiver enabled',receiver.checked&&v.physicalNativeReceiverEnabled);
   const refined=await digest(v.nativePreviewPixels());check('Actual geometry replaces artwork receiver',refined!==beforeImage,{refined,original:beforeImage});
   check('Static receiver render restores scene poses and visibility',sceneState()===beforeState);
   await capture('b05-refined-map-tick74');
   receiver.click();check('Original receiver restores exact bytes',!v.physicalNativeReceiverEnabled&&await digest(v.nativePreviewPixels())===beforeImage);
 }

 if(cfg.cropRegion){
   const r=cfg.cropRegion, origin=cfg.sourceOrigin;
   check('Source crop coordinates are explicit',origin[0]===0&&origin[1]===0&&r.x===888&&r.y===900&&r.width===96&&r.height===108,{region:r,origin,camera:v.bindings.document().camera});
   window.nativeEffectCrops=[];
   for(const tick of [0,74]){
     v.seekNativeArt(tick,focus);
     const images=[];
     for(const presentation of ['original-art','physical-over-art','refined-map']){
       if(presentation==='original-art'){if(checkbox.checked)checkbox.click();}
       else {if(!checkbox.checked)checkbox.click();await wait(()=>document.querySelector('[aria-label="Refined map background"]'),'refined receiver control after effects enable');const input=document.querySelector('[aria-label="Refined map background"]');const refined=presentation==='refined-map';if(input.checked!==refined)input.click();}
       const pixels=v.nativePreviewPixels(), data=new Uint8ClampedArray(r.width*r.height*4);
       check('Crop inside actual buffer '+presentation+' '+tick,r.x-origin[0]>=0&&r.y-origin[1]>=0&&r.x-origin[0]+r.width<=pixels.width&&r.y-origin[1]+r.height<=pixels.height);
       for(let y=0;y<r.height;y++){const start=((r.y-origin[1]+y)*pixels.width+r.x-origin[0])*4;data.set(pixels.data.subarray(start,start+r.width*4),y*r.width*4);}
       const canvas=document.createElement('canvas');canvas.width=r.width;canvas.height=r.height;canvas.getContext('2d').putImageData(new ImageData(data,r.width,r.height),0,0);
       const row={name:'b05-crop-'+presentation+'-tick'+tick,tick,presentation,region:r,origin,camera:v.bindings.document().camera,buffer:{width:pixels.width,height:pixels.height,sha256:await digest(pixels)},crop_sha256:await digest({data}),png:canvas.toDataURL('image/png')};
       window.nativeEffectCrops.push(row);images.push(row);
     }
     const panel=document.createElement('div');panel.style.cssText='position:fixed;inset:0;z-index:99999;background:#202020;color:white;padding:16px;box-sizing:border-box;font:14px sans-serif';
     const title=document.createElement('p');title.textContent='Actual source-camera buffer crops · tick '+tick+' · source pixels (888,900), 96×108 · 3× nearest-neighbor';panel.append(title);
     const columns=document.createElement('div');columns.style.cssText='display:flex;gap:12px';panel.append(columns);
     for(const row of images){const col=document.createElement('div'),label=document.createElement('p'),img=document.createElement('img');label.textContent=row.presentation;img.src=row.png;img.style.cssText='width:288px;height:324px;image-rendering:pixelated';col.append(label,img);columns.append(col);await img.decode();}
     document.body.append(panel);await capture('b05-comparison-crop-tick'+tick);panel.remove();
   }
   const input=document.querySelector('[aria-label="Refined map background"]');if(input.checked)input.click();
 }

 v.seekNativeArt(0,focus);checkbox.click();check('Checkbox disables substitution',!v.physicalNativeEffectsEnabled&&await digest(v.nativePreviewPixels())===art);
 await capture('b05-original-art-tick0');
 select('State preview view','initial');await wait(()=>v.statePresentationMode==='physical','ordinary physical mode');
 check('Ordinary physical orbit remains available',!!v.orbit.enabled&&!v.nativeSurface);
 select('State preview view','art');
 select('Mission','');await wait(()=>!v.bindings.document()?.mission?.importedFrom,'mission retired');
 check('Physical substitution retires with mission',!v.physicalNativeEffects&&!v.physicalNativeEffectsEnabled);
 await new Promise(r=>setTimeout(r,100));
 select('Mission',cfg.entry.mission);
 await wait(()=>document.querySelector('[aria-label="State preview asset"] option[value="'+cfg.entry.id+'"]'),'mission UI recovers after retirement');
 select('State preview asset',cfg.entry.id);
 await wait(()=>v.loopClocks.ready&&v.missionStates.players.size===1&&v.physicalNativeEffects,'physical preview rebinds after retirement');
 check('Reactive UI survives mission retirement and rebind',v.nativeArtStatus().ready);
 select('Mission','');await wait(()=>!v.bindings.document()?.mission?.importedFrom,'second mission retirement');
 await new Promise(r=>setTimeout(r,100));

 return {status:'SCATTER_EXECUTED_PENDING_VISUAL_REVIEW',checks,states,scope:'Real UI and source-bound saved B05 geometry on native artwork; no geometry approval, actor crossing, or whole-map closure. Refined receiver is exercised only when requested.'};
})()
