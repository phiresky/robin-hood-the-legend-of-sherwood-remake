(async()=>{
const v=window.reviewViewport,T=window.reviewThree,cfg=window.stateUiPacket,checks=[],evidence=window.stateUiEvidence={checks,captures:[],controls:[]};
const check=(name,pass)=>{checks.push({name,pass:!!pass});if(!pass)throw Error(name)};
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(fn,label)=>{for(let i=0;i<2400;i++){if(await fn())return;await sleep(100)}throw Error('Timeout '+label)};
const picker=label=>document.querySelector('select[aria-label="'+label+'"]');
const offered=(label,value)=>{const p=picker(label);return !!p&&[...p.options].some(o=>o.value===value)};
const select=(label,value)=>{check('Offered '+label+' '+value,offered(label,value));const p=picker(label);p.value=value;p.dispatchEvent(new Event('change',{bubbles:true}))};
const mission=async name=>{if((v.bindings.document()?.mission?.importedFrom??'')===name)return;select('Mission',name);await until(()=>(v.bindings.document()?.mission?.importedFrom??'')===name,'mission '+name)};
const mode=async(name,row)=>{select('State preview view',name);await until(()=>v.stateDelivery.ready&&(name==='art'?v.stateDelivery.mode==='native-art':v.stateDelivery.mode==='physical-endpoint'&&v.deliveryEndpoint===name),'mode '+name)};
const capture=async name=>{await sleep(200);window.scatterStage={name,time:Date.now()};await until(()=>window.scatterCaptures[name],'capture '+name);evidence.captures.push(name)};
const button=[...document.querySelectorAll('.editor-modes button')].find(b=>b.textContent.trim()==='Mission');check('Mission mode available',button);button.click();
for(const row of cfg.rows){
 await until(()=>offered('Mission',row.mission),'mission option');await mission(row.mission);await until(()=>offered('State preview asset',row.id),'climbing catalog entry');select('State preview asset',row.id);await until(()=>v.stateDelivery.ready&&v.deliveryFamily===row.family&&offered('State preview view','initial'),'climbing loaded');
 check('No unrelated aperture adopted',!v.aperturePreview);v.setTextureDisplay(true,true);await mode('initial',row);check('Initial physical model present',v.stateDelivery.physical.children[0]?.children.length===1);const oldRoot=v.stateDelivery.physical.children[0];
 if(row===cfg.rows[0]){const box=new T.Box3().setFromObject(oldRoot);check('Initial bounds nonempty',!box.isEmpty());const target=box.getCenter(new T.Vector3()),size=box.getSize(new T.Vector3()),span=Math.max(160,size.x*1.5,size.y*1.5,size.z*1.5);v.setPerspective(0);v.applyState(v.lookState(target.clone().add(new T.Vector3(0,Math.sin(35*Math.PI/180)*1000,Math.cos(35*Math.PI/180)*1000)),target,span));await capture('climbing-catalog-initial-source-camera')}
 await mode('applied',row);check('Applied physical model present',v.stateDelivery.physical.children[1]?.children.length===1);if(row===cfg.rows[0])await capture('climbing-catalog-applied-source-camera');
 await mode('art',row);check('Native mode hides physical endpoints',!v.stateDelivery.physical.visible);const reset=[...document.querySelectorAll('[aria-label="State preview"] button')].find(b=>b.textContent.trim()==='Reset');check('Reset control present',reset);reset.click();check('Reset clears transition clock',v.stateDelivery.familyTick(row.family)===undefined);evidence.controls.push(row.id);
}
await mission('');await until(()=>!v.stateDelivery.ready,'map-only retirement');check('Endpoints detached on retirement',v.stateDelivery.physical.children.length===0);check('Both new controls exercised',evidence.controls.length===2);
return{status:'SCATTER_EXECUTED_PENDING_VISUAL_REVIEW',checks,evidence,scope:'Two additive climbing catalog controls through actual normal editor UI. Prior4200-check endpoint/native timeline proof retains its narrower scope; no static ownership or gameplay certification.'};
})()
