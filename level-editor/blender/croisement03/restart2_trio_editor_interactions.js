(async () => {
 const v=window.reviewViewport,T=window.reviewThree,cfg=window.c3InteractionConfig;
 const e=window.c3InteractionEvidence={checks:[],selection:[],captures:[],inserted:[],http:[]};
 const check=(name,value,detail)=>{e.checks.push({name,pass:!!value,detail});if(!value)throw Error(name+': '+JSON.stringify(detail));};
 const sleep=ms=>new Promise(r=>setTimeout(r,ms));
 const until=async(test,label)=>{for(let i=0;i<2400;i++){if(await test())return;await sleep(100);}throw Error('Timeout '+label);};
 const stable=x=>Array.isArray(x)?x.map(stable):x&&typeof x==='object'?Object.fromEntries(Object.keys(x).sort().map(k=>[k,stable(x[k])])):x;
 const same=(a,b)=>JSON.stringify(stable(a))===JSON.stringify(stable(b));
 const capture=async name=>{window.scatterStage={name,time:Date.now()};await until(()=>window.scatterCaptures?.[name],name);e.captures.push(name);};
 check('Actual editor fixture available',v&&T&&cfg);
 const initial=structuredClone(v.bindings.document());e.originalDocument=initial;
 check('Exact original group and part counts',initial.groups.length===86&&initial.objects.length===127);
 const refs=d=>[...(d.assetSources??[]),...(d.sceneAssets??[])];
 const pins=(d,stored=false)=>{for(const[id,pin]of Object.entries(cfg.assets)){const ref=refs(d).find(r=>r.id===id);check('Exact source pins '+id,ref?.model===pin.model&&ref?.descriptor===pin.descriptor&&ref?.model_sha256===pin.model_sha256&&ref?.descriptor_sha256===pin.descriptor_sha256,ref);check('Descriptor-derived scene '+id,(stored&&ref.model_scene===undefined)||(ref.model_scene??'default')===(pin.model_scene??'default'),ref);check('Descriptor-derived resources '+id,(stored&&ref.resources===undefined)||same(ref.resources??[],pin.resources??[]),ref);}};
 pins(initial);
 for(const item of cfg.insertions){
  const group=initial.groups.find(g=>g.id===item.asset);check('Original group exists '+item.asset,!!group);
  const parts=initial.objects.filter(p=>p.group===group.id);
  check('Exact original part membership '+item.asset,same(parts.map(p=>p.node.split(':').at(-1)).sort(),[...cfg.parts[item.asset]].sort()));
  const groupRow=()=>[...document.querySelectorAll('.object-list li.depth-0')].find(r=>r.title===(group.name??group.id));
  const row=groupRow();check('Group UI row '+group.id,!!row);row.click();await until(()=>v.bindings.selection()?.kind==='group'&&v.bindings.selection()?.id===group.id,'group selection');
  groupRow().querySelector('.chev-btn').click();
  const children=()=>{const result=[];for(let r=groupRow().nextElementSibling;r?.classList.contains('depth-1');r=r.nextElementSibling)result.push(r);return result;};
  await until(()=>children().length===parts.length,'expanded parts');
  for(const part of parts){const r=children().find(r=>r.title===(part.name??part.id));check('Part UI row '+part.id,!!r);r.click();await until(()=>v.bindings.selection()?.id===part.id,'part selection');e.selection.push({group:group.id,part:part.id,node:part.node});}
  groupRow().querySelector('.chev-btn').click();
 }
 v.select(null);await capture('c3-trio-editor-selected-parts');
 const nativeDescriptor=cfg.assets['croisement03-native-arbre06'].descriptor_sha256;
 const nativeEffects=()=>[...v.scenery.effects.values()].filter(x=>x.descriptorSha256===nativeDescriptor);
 await until(()=>nativeEffects().length===1&&nativeEffects()[0].frames?.frames.length===14,'original native owner');
 for(const item of cfg.insertions){
  const search=document.querySelector('[aria-label="Find assets"]'),source=document.querySelector('[aria-label="Source level"]');check('Palette filter controls',search&&source);
  source.value='Croisement03';source.dispatchEvent(new Event('change',{bubbles:true}));search.value=item.asset;search.dispatchEvent(new Event('input',{bubbles:true}));
  await until(()=>document.querySelector(`[data-asset-id="${item.asset}"]`),'palette '+item.asset);
  const card=document.querySelector(`[data-asset-id="${item.asset}"]`);card.scrollIntoView({block:'center'});
  await until(()=>card.querySelector('canvas')&&!card.querySelector('.preview-status')?.textContent,'loaded palette preview '+item.asset);await capture('c3-palette-'+item.asset);
  const before=new Set(v.bindings.document().groups.map(g=>g.id));const add=card.querySelector('button[aria-label^="Add "]');check('Palette insertion enabled '+item.asset,add&&!add.disabled);add.click();
  await until(()=>v.bindings.document().groups.length===before.size+1,'insert '+item.asset);
  const inserted=v.bindings.document().groups.find(g=>!before.has(g.id));check('Unique inserted group '+item.asset,!!inserted);
  const input=document.querySelector('.object-detail .transform-fields input[aria-label="X"]');check('Inserted X editor '+item.asset,!!input);input.value=String(item.x);input.dispatchEvent(new Event('change',{bubbles:true}));
  await until(()=>v.bindings.document().groups.find(g=>g.id===inserted.id)?.transform.dx===item.x,'independent pose');
  const after=v.bindings.document(),children=after.objects.filter(p=>p.group===inserted.id);
  check('Inserted exact parts '+item.asset,same(children.map(p=>p.node.split(':').at(-1)).sort(),[...cfg.parts[item.asset]].sort())&&children.every(p=>p.node.startsWith('asset:'+item.asset+':')));
  e.inserted.push({asset:item.asset,id:inserted.id,x:item.x});
 }
 const changed=structuredClone(v.bindings.document());
 check('All original groups unchanged',same(changed.groups.filter(g=>initial.groups.some(o=>o.id===g.id)),initial.groups));
 check('All original objects unchanged',same(changed.objects.filter(p=>initial.objects.some(o=>o.id===p.id)),initial.objects));
 check('All original asset refs unchanged',same(changed.assetSources,initial.assetSources)&&same(changed.sceneAssets,initial.sceneAssets));pins(changed);
 await until(()=>nativeEffects().length===2&&nativeEffects().every(x=>x.frames?.frames.length===14),'one native effect per placement');
 check('Exactly two native runtime owners after one insertion',nativeEffects().length===2);
 const save=[...document.querySelectorAll('button')].find(b=>b.textContent.trim().startsWith('Save'));check('Save enabled',save&&!save.disabled);save.click();
 await until(()=>[...document.querySelectorAll('button')].some(b=>b.textContent.trim()==='Save'&&b.disabled),'saved map');
 const maps=await(await(await navigator.storage.getDirectory()).getDirectoryHandle('sherwood-level-editor')).getDirectoryHandle('maps');
 const stored=JSON.parse(await(await(await maps.getFileHandle('croisement03.rhlos-map.json')).getFile()).text());e.savedDocument=stored;
 const originalIds=new Set(cfg.stage_map.placements.map(p=>p.id));
 check('All stored original placements preserved',stored.placements.filter(p=>originalIds.has(p.id)).length===originalIds.size&&cfg.stage_map.placements.every(p=>{const q=stored.placements.find(q=>q.id===p.id);return q&&Object.entries(p).every(([key,value])=>same(q[key],value));}));
 for(const inserted of e.inserted)check('Saved independent pose '+inserted.asset,stored.placements.some(p=>p.id===inserted.id&&p.transform.dx===inserted.x));pins(stored,true);
 await Promise.all(window.observedPublicationPending??[]);
 for(const[id,pin]of Object.entries(cfg.assets)){const path='/library/'+pin.model;const rows=(window.observedPublicationResponses??[]).filter(r=>r.path===path);check('Actual original GLB bytes '+id,rows.length>0&&rows.every(r=>r.sha256===pin.model_sha256),rows);e.http.push(...rows);}
 const bounds=new T.Box3();for(const item of e.inserted){const wrapper=v.groupViews.get(item.id).wrapper;wrapper.updateWorldMatrix(true,true);bounds.union(new T.Box3().setFromObject(wrapper));}const center=bounds.getCenter(new T.Vector3()),size=bounds.getSize(new T.Vector3()),el=initial.camera.elevation_deg*Math.PI/180;v.select(null);v.setPerspective(0);v.applyState(v.lookState(center.clone().addScaledVector(new T.Vector3(0,Math.sin(el),Math.cos(el)),Math.max(400,size.length()*2)),center,Math.max(size.x,size.y,size.z)*.7));await sleep(200);
 for(const[id,pin]of Object.entries(cfg.assets)){const descriptor=v.scenery.descriptors.get(id);check('Actual catalog descriptor scene/resources '+id,descriptor&&((descriptor.model_scene??'default')===(pin.model_scene??'default'))&&same(descriptor.resources??[],pin.resources??[]));for(const resource of pin.resources??[]){const rows=window.observedPublicationResponses.filter(r=>r.path==='/library/'+resource.path);check('Actual resource bytes '+resource.path,rows.length>0&&rows.every(r=>r.sha256===resource.sha256),rows);}}const catalogs=window.observedPublicationResponses.filter(r=>r.path==='/library/3d-assets/index.json');check('Actual pinned catalog bytes',catalogs.length>0&&catalogs.every(r=>r.sha256===cfg.catalog_sha256),catalogs);
 await capture('c3-trio-editor-saved-insertions');
 e.reloadExpectation={groups:changed.groups,objects:changed.objects,assetSources:changed.assetSources,sceneAssets:changed.sceneAssets,inserted:e.inserted,assets:cfg.assets,catalogSha256:cfg.catalog_sha256};window.c3InteractionReloadExpectation=structuredClone(e.reloadExpectation);
 return {status:'EDITOR_INTERACTIONS_EXECUTED_RELOAD_PENDING',checks:e.checks,evidence:e};
})()
