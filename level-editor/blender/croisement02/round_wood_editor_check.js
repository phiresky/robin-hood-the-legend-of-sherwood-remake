(async()=>{
 const v=window.reviewViewport,T=window.reviewThree,cfg=window.tree02Config,checks=[];
 const evidence=window.tree02Evidence={checks,captures:[],selection:[],poses:{}};
 const check=(name,value,detail)=>{checks.push({name,pass:!!value,detail});if(!value)throw Error(name+': '+JSON.stringify(detail));};
 const sleep=ms=>new Promise(r=>setTimeout(r,ms));
 const until=async(test,label)=>{for(let i=0;i<2400;i++){if(await test())return;await sleep(100)}throw Error('Timeout '+label)};
 const capture=async(name)=>{window.scatterStage={name,time:Date.now()};await until(()=>window.scatterCaptures[name],name);evidence.captures.push(name)};
 const stable=x=>Array.isArray(x)?x.map(stable):x&&typeof x==='object'?Object.fromEntries(Object.keys(x).sort().map(k=>[k,stable(x[k])])):x;
 const same=(a,b)=>JSON.stringify(stable(a))===JSON.stringify(stable(b));
 const initial=structuredClone(v.bindings.document());evidence.originalDocument=initial;
 check('Full map group count',initial.groups.length===cfg.expected_groups,initial.groups.length);
 for(const id of cfg.asset_ids){
  const pin=cfg.pins[id],ref=initial.assetSources.find(x=>x.id===id);check(id+' saved source pins',ref?.model_sha256===pin.model_sha256&&ref.descriptor_sha256===pin.descriptor_sha256,ref);
  const group=initial.groups.find(x=>x.id===id);check(id+' group exists',!!group);check(id+' placement unchanged',same(group.transform,cfg.placements[id].transform));
  const children=initial.objects.filter(x=>x.group===group.id);check(id+' exact parts',same(children.map(p=>p.node.split(':').at(-1)).sort(),[...cfg.expected_parts[id]].sort()));
  const groupRow=()=>[...document.querySelectorAll('.object-list li.depth-0')].find(row=>row.title===(group.name??group.id));
  let row=groupRow();check('Group UI row '+id,!!row);row.click();await until(()=>v.bindings.selection()?.kind==='group'&&v.bindings.selection()?.id===id,'group selection');groupRow().querySelector('.chev-btn').click();
  const childRows=()=>{const rows=[];for(let c=groupRow().nextElementSibling;c?.classList.contains('depth-1');c=c.nextElementSibling)rows.push(c);return rows};
  await until(()=>childRows().length===children.length,'expanded children');
  for(const child of children){const r=childRows().find(x=>x.title===(child.name??child.id));check('Part UI row '+child.id,!!r);r.click();await until(()=>v.bindings.selection()?.id===child.id,'part selection');evidence.selection.push({group:id,part:child.id});}
  groupRow().querySelector('.chev-btn').click();v.select(null);
  const wrapper=v.groupViews.get(id).wrapper;wrapper.updateWorldMatrix(true,true);const box=new T.Box3().setFromObject(wrapper),center=box.getCenter(new T.Vector3()),size=box.getSize(new T.Vector3());
  const elevation=initial.camera.elevation_deg*Math.PI/180;
  const frame=(angle,close=false)=>{const target=close?new T.Vector3(center.x,box.min.y+65,center.z):center;const dir=new T.Vector3(Math.sin(angle)*Math.cos(elevation),Math.sin(elevation),Math.cos(angle)*Math.cos(elevation));v.setPerspective(0);v.applyState(v.lookState(target.clone().addScaledVector(dir,Math.max(400,size.length()*2)),target,close?145:Math.max(size.x,size.y,size.z)*.7));};
  for(const [angle,name]of [[0,'source'],[Math.PI,'reverse']]){frame(angle);await sleep(250);await capture(id+'-context-'+name);frame(angle,true);await sleep(250);await capture(id+'-root-contact-'+name);}
 }
 const inserted=[];
 for(const [i,id]of cfg.asset_ids.entries()){
  const source=document.querySelector('[aria-label="Source level"]'),search=document.querySelector('[aria-label="Find assets"]');source.value='Croisement02';source.dispatchEvent(new Event('change',{bubbles:true}));search.value=id;search.dispatchEvent(new Event('input',{bubbles:true}));
  await until(()=>document.querySelector('[data-asset-id="'+id+'"]'),'palette card');const card=document.querySelector('[data-asset-id="'+id+'"]');card.scrollIntoView({block:'center'});await until(()=>card.querySelector('canvas')&&!card.querySelector('.preview-status')?.textContent,'preview');await capture(id+'-palette');
  const add=card.querySelector('button[aria-label^="Add "]');check(id+' insert enabled',add&&!add.disabled);add.click();await until(()=>v.bindings.document().groups.length===initial.groups.length+i+1,'palette insertion');
  const group=v.bindings.document().groups.at(-1),x=2400+i*400,input=document.querySelector('.object-detail .transform-fields input[aria-label="X"]');check('Inserted X input',!!input);input.value=String(x);input.dispatchEvent(new Event('change',{bubbles:true}));await until(()=>v.bindings.document().groups.at(-1).transform.dx===x,'independent pose');inserted.push({id:group.id,x});
  check(id+' inserted source identity',v.bindings.document().objects.filter(p=>p.group===group.id).every(p=>p.node.startsWith('asset:'+id+':')));
 }
 const changed=structuredClone(v.bindings.document());check('Original groups preserved',same(changed.groups.slice(0,initial.groups.length),initial.groups));check('Original parts preserved',same(changed.objects.filter(x=>initial.objects.some(o=>o.id===x.id)),initial.objects));
 const save=[...document.querySelectorAll('button')].find(x=>x.textContent.trim().startsWith('Save'));check('Save enabled',save&&!save.disabled);save.click();await until(()=>[...document.querySelectorAll('button')].some(x=>x.textContent.trim()==='Save'&&x.disabled),'saved');
 const maps=await(await(await navigator.storage.getDirectory()).getDirectoryHandle('sherwood-level-editor')).getDirectoryHandle('maps');const stored=JSON.parse(await(await(await maps.getFileHandle('croisement02.rhlos-map.json')).getFile()).text());
 for(const p of inserted)check('Saved inserted pose '+p.id,stored.placements.some(x=>x.id===p.id&&x.transform.dx===p.x));
 await capture('both-trees-saved');evidence.savedDocument=stored;evidence.reloadExpectation={groups:changed.groups,objects:changed.objects,sourcePins:cfg.pins,insertedGroup:inserted.at(-1).id,insertedX:inserted.at(-1).x};
 return {status:'SCATTER_EXECUTED_PENDING_VISUAL_REVIEW',checks,evidence,scope:'Full private Croisement02 map. Both trees group/parts, original and reverse root contacts, palette insertions and save. Page reload required next.'};
})()
