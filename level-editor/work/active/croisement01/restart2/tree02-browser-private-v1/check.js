(async()=>{
 const v=window.reviewViewport,T=window.reviewThree,cfg=window.tree02Config,checks=[];
 const evidence=window.tree02Evidence={checks,captures:[],selection:[],http:[],poses:{}};
 const check=(name,value,detail)=>{checks.push({name,pass:!!value,detail});if(!value)throw Error(name+': '+JSON.stringify(detail));};
 const sleep=ms=>new Promise(r=>setTimeout(r,ms));
 const until=async(test,label)=>{for(let i=0;i<2400;i++){if(await test())return;await sleep(100)}throw Error('Timeout '+label)};
 const capture=async(name)=>{window.scatterStage={name,time:Date.now()};await until(()=>window.scatterCaptures[name],name);evidence.captures.push(name)};
 const stable=x=>Array.isArray(x)?x.map(stable):x&&typeof x==='object'?Object.fromEntries(Object.keys(x).sort().map(k=>[k,stable(x[k])])):x;
 const same=(a,b)=>JSON.stringify(stable(a))===JSON.stringify(stable(b));
 const initial=structuredClone(v.bindings.document());evidence.originalDocument=initial;check('Expected staged group and object counts',initial.groups.length===cfg.expected_counts.groups&&initial.objects.length===cfg.expected_counts.objects,{groups:initial.groups.length,objects:initial.objects.length});
 check('Every grouped placement is an independent group',initial.groups.length===cfg.stage_map.placements.filter(x=>!x.ungrouped).length,initial.groups.length);check('Ungrouped terrace preserved',initial.objects.filter(x=>!x.group).length===1);
 for(const [id,pin]of Object.entries(cfg.proof.assets)){
  const ref=initial.assetSources.find(x=>x.id===id);check(id+' exact saved source pins',ref?.model_sha256===pin.model_sha256&&ref.descriptor_sha256===pin.descriptor_sha256,ref);
 }
 await Promise.all(window.observedPublicationPending);
 for(const [id,pin]of Object.entries(cfg.proof.assets)){
  const path='/library/3d-assets/croisement01/'+id+'/model.glb';const rows=window.observedPublicationResponses.filter(x=>x.path===path);check(id+' actual staged indexed model bytes',rows.length>0&&rows.every(x=>x.sha256===pin.model_sha256),rows);evidence.http.push(...rows);
 }
 const tree=initial.groups.find(x=>x.id==='tree02-approved');check('Tree02 group exists',!!tree);
 const parts=initial.objects.filter(x=>x.group===tree.id);check('Exact approved Tree02 parts',same(parts.map(p=>p.node.split(':').at(-1)).sort(),[...cfg.tree_parts].sort()),parts);check('Inferred crown carries scenery identity',parts.find(p=>p.node.endsWith(':foliage-tree02-inferred-crown'))?.kind==='scenery',parts);
 const retained=initial.groups.find(x=>initial.objects.filter(p=>p.group===x.id).some(p=>p.node==='asset:croisement01-group-005:building-005'));check('Retained group005 exists',!!retained);
 check('Retained group has exact five parts',same(initial.objects.filter(p=>p.group===retained.id).map(p=>p.node.split(':').at(-1)).sort(),[...cfg.proof.retained_parts].sort()));
 check('Original tree placement preserved',same(tree.transform,cfg.proof.added_placements[0].transform),tree.transform);
 for(const group of [tree,retained]){
  const groupRow=()=>[...document.querySelectorAll('.object-list li.depth-0')].find(row=>row.title===(group.name??group.id));let row=groupRow();check('Selection row exists '+group.id,!!row);row.click();await until(()=>v.bindings.selection()?.kind==='group'&&v.bindings.selection()?.id===group.id,'select '+group.id);
  row=groupRow();row.querySelector('.chev-btn').click();
  const childRows=()=>{const rows=[];for(let c=groupRow().nextElementSibling;c?.classList.contains('depth-1');c=c.nextElementSibling)rows.push(c);return rows};
  const children=initial.objects.filter(x=>x.group===group.id);await until(()=>childRows().length===children.length,'parts expanded');
  for(const child of children){const row=childRows().find(r=>r.title===(child.name??child.id));check('Named part selection row '+child.id,!!row);row.click();await until(()=>v.bindings.selection()?.id===child.id,'part selection');evidence.selection.push({group:group.id,part:child.id});}
  groupRow().querySelector('.chev-btn').click();
 }
 v.select(null);
 const wrapper=v.groupViews.get(tree.id).wrapper;wrapper.updateWorldMatrix(true,true);const box=new T.Box3().setFromObject(wrapper),center=box.getCenter(new T.Vector3()),size=box.getSize(new T.Vector3());evidence.poses.originalBounds=[box.min.toArray(),box.max.toArray()];
 const elevation=initial.camera.elevation_deg*Math.PI/180;const frame=angle=>{const dir=new T.Vector3(Math.sin(angle)*Math.cos(elevation),Math.sin(elevation),Math.cos(angle)*Math.cos(elevation));v.setPerspective(0);v.applyState(v.lookState(center.clone().addScaledVector(dir,Math.max(400,size.length()*2)),center,Math.max(size.x,size.y,size.z)*.7));};
 frame(0);await sleep(350);await capture('tree02-context-source');
 const visibility=[...v.groupViews.values()].map(x=>[x.wrapper,x.wrapper.visible]);for(const [o]of visibility)o.visible=o===wrapper;
 for(let i=0;i<8;i++){frame(i*Math.PI/4);await sleep(100);await capture('tree02-isolated-view-'+i);}
 for(const[o,visible]of visibility)o.visible=visible;
 frame(Math.PI);await sleep(100);await capture('tree02-context-reverse');
 const search=document.querySelector('[aria-label="Find assets"]'),source=document.querySelector('[aria-label="Source level"]');source.value='Croisement01';source.dispatchEvent(new Event('change',{bubbles:true}));search.value='croisement01-tree-02';search.dispatchEvent(new Event('input',{bubbles:true}));
 await until(()=>document.querySelector('[data-asset-id="croisement01-tree-02"]'),'tree palette');const card=document.querySelector('[data-asset-id="croisement01-tree-02"]');card.scrollIntoView({block:'center'});await until(()=>card.querySelector('canvas')&&!card.querySelector('.preview-status')?.textContent,'palette preview');await capture('tree02-palette');
 const add=card.querySelector('button[aria-label^="Add "]');check('Palette exact identity and enabled insertion',add&&!add.disabled);add.click();await until(()=>v.bindings.document().groups.length===initial.groups.length+1,'insert tree');
 const inserted=v.bindings.document().groups.at(-1);const input=document.querySelector('.object-detail .transform-fields input[aria-label="X"]');check('Inserted tree transform UI',!!input);input.value='2400';input.dispatchEvent(new Event('change',{bubbles:true}));await sleep(200);
 const changed=structuredClone(v.bindings.document());check('Original groups preserved after insertion',same(changed.groups.slice(0,initial.groups.length),initial.groups));check('Original objects preserved after insertion',same(changed.objects.filter(x=>initial.objects.some(o=>o.id===x.id)),initial.objects));check('Inserted model source identity',changed.objects.filter(x=>x.group===inserted.id).every(x=>x.node.startsWith('asset:croisement01-tree-02:')));
 check('Inserted independent pose',changed.groups.at(-1).transform.dx===2400,changed.groups.at(-1));evidence.poses.inserted=changed.groups.at(-1);
 const save=[...document.querySelectorAll('button')].find(x=>x.textContent.trim().startsWith('Save'));check('Save is enabled after pose edit',save&&!save.disabled);save.click();await until(()=>[...document.querySelectorAll('button')].some(x=>x.textContent.trim()==='Save'&&x.disabled),'save');
 const maps=await(await(await navigator.storage.getDirectory()).getDirectoryHandle('sherwood-level-editor')).getDirectoryHandle('maps');const stored=JSON.parse(await(await(await maps.getFileHandle('croisement01.rhlos-map.json')).getFile()).text());
 evidence.savedDocument=stored;check('Saved inserted placement pose',stored.placements.some(x=>x.id===inserted.id&&x.transform.dx===2400),stored.placements.at(-1));
 for(const [id,pin]of Object.entries(cfg.proof.assets)){const ref=stored.assetSources.find(x=>x.id===id);check('Saved reload source hashes '+id,ref?.model_sha256===pin.model_sha256&&ref.descriptor_sha256===pin.descriptor_sha256);}
 await capture('tree02-saved-insertion');
 evidence.reloadExpectation={groups:changed.groups,objects:changed.objects,sourcePins:cfg.proof.assets,insertedGroup:inserted.id,insertedX:2400};window.tree02ReloadExpectation=structuredClone(evidence.reloadExpectation);
 return {status:'SCATTER_EXECUTED_PENDING_VISUAL_REVIEW',checks,evidence,scope:'Private staged library: approved Tree02 lower/upper wood and inferred crown, with five retained group005 parts. Full page reload remains a separate required check.'};
})()
