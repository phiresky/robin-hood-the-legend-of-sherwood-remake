(async()=>{
 const expected=window.c3InteractionReloadExpectation;if(!expected)throw Error('Inject saved expectation from pre-reload result');
 const checks=[],check=(name,value,detail)=>{checks.push({name,pass:!!value,detail});if(!value)throw Error(name+': '+JSON.stringify(detail));};
 const sleep=ms=>new Promise(r=>setTimeout(r,ms)),until=async(test,label)=>{for(let i=0;i<2400;i++){if(await test())return;await sleep(100);}throw Error('Timeout '+label);};
 const stable=x=>Array.isArray(x)?x.map(stable):x&&typeof x==='object'?Object.fromEntries(Object.keys(x).sort().map(k=>[k,stable(x[k])])):x,same=(a,b)=>JSON.stringify(stable(a))===JSON.stringify(stable(b));
 await until(()=>window.reviewViewport?.bindings.document()?.groups.some(g=>g.id===expected.inserted.at(-1).id),'normal saved-map reload');
 const v=window.reviewViewport,doc=structuredClone(v.bindings.document());
 for(const field of ['groups','objects','assetSources','sceneAssets'])check('Reload exact '+field,same(doc[field],expected[field]));
 for(const item of expected.inserted){const group=doc.groups.find(g=>g.id===item.id);check('Reload independent pose '+item.asset,group?.transform.dx===item.x);const row=[...document.querySelectorAll('.object-list li.depth-0')].find(r=>r.title===(group.name??group.id));check('Reload actual UI row '+item.asset,!!row);row.click();await until(()=>v.bindings.selection()?.kind==='group'&&v.bindings.selection()?.id===group.id,'reload selection');}
 const descriptor=expected.assets['croisement03-native-arbre06'].descriptor_sha256;
 await until(()=>[...v.scenery.effects.values()].filter(e=>e.descriptorSha256===descriptor&&e.frames?.frames.length===14).length===2,'reload two native owners');
 await Promise.all(window.observedPublicationPending??[]);
 for(const[id,pin]of Object.entries(expected.assets)){const rows=(window.observedPublicationResponses??[]).filter(r=>r.path==='/library/'+pin.model);check('Reload exact fetched model '+id,rows.length>0&&rows.every(r=>r.sha256===pin.model_sha256),rows);}
 const T=window.reviewThree,bounds=new T.Box3();for(const item of expected.inserted){const wrapper=v.groupViews.get(item.id).wrapper;wrapper.updateWorldMatrix(true,true);bounds.union(new T.Box3().setFromObject(wrapper));}const center=bounds.getCenter(new T.Vector3()),size=bounds.getSize(new T.Vector3()),el=doc.camera.elevation_deg*Math.PI/180;v.select(null);v.setPerspective(0);v.applyState(v.lookState(center.clone().addScaledVector(new T.Vector3(0,Math.sin(el),Math.cos(el)),Math.max(400,size.length()*2)),center,Math.max(size.x,size.y,size.z)*.7));await sleep(200);
 window.scatterStage={name:'c3-trio-editor-reloaded-insertions',time:Date.now()};await until(()=>window.scatterCaptures?.['c3-trio-editor-reloaded-insertions'],'reload capture');
 return {status:'RELOAD_EXECUTED_PENDING_VISUAL_REVIEW',checks,document:doc,capture:'c3-trio-editor-reloaded-insertions'};
})()
