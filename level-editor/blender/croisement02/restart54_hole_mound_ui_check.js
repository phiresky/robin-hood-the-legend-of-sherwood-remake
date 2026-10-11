(async()=>{
const v=window.reviewViewport,T=window.reviewThree,cfg=window.stateUiPacket,checks=[],evidence=window.stateUiEvidence={checks,captures:[],controls:[],missions:[],expectedErrors:[]};
const check=(name,pass,detail)=>{checks.push({name,pass:!!pass,...(detail===undefined?{}:{detail})});if(!pass)throw Error(name+': '+JSON.stringify(detail))};
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(test,label)=>{for(let i=0;i<2400;i++){if(await test())return;const error=document.querySelector('#result')?.textContent;if(error?.startsWith('FAIL'))throw Error(error);await sleep(100)}throw Error('Timeout '+label)};
const select=(label,value)=>{const input=document.querySelector('select[aria-label="'+label+'"]');check('Selector '+label,!!input);check('Option '+value,[...input.options].some(o=>o.value===value));input.value=value;input.dispatchEvent(new Event('change',{bubbles:true}))};
const same=(a,b)=>JSON.stringify(a)===JSON.stringify(b),visible=node=>{for(let n=node;n;n=n.parent)if(!n.visible)return false;return true};
const hash=async data=>[...new Uint8Array(await crypto.subtle.digest('SHA-256',new Uint8Array(data)))].map(n=>n.toString(16).padStart(2,'0')).join('');
const capture=async(name,object)=>{v.setPerspective(0);const box=new T.Box3().setFromObject(object);check('Capture has geometry '+name,!box.isEmpty());const target=box.getCenter(new T.Vector3()),size=box.getSize(new T.Vector3());const span=Math.max(120,size.x*1.8,size.z*1.8);v.applyState(v.lookState(target.clone().add(new T.Vector3(0,Math.sin(35*Math.PI/180)*1000,Math.cos(35*Math.PI/180)*1000)),target,span));await sleep(200);window.scatterStage={name,time:Date.now()};await until(()=>window.scatterCaptures[name],'capture '+name);evidence.captures.push(name)};
const sampling=root=>{let masks=0,rgb=0;root.traverse(o=>{if(!o.isMesh)return;for(const m of Array.isArray(o.material)?o.material:[o.material])if(m.userData.export_complementary_binary_ownership){masks++;check('Binary mask nearest',m.map?.magFilter===T.NearestFilter&&m.map?.minFilter===T.NearestFilter&&!m.map?.generateMipmaps);if(m.userData.export_rgb_sampling==='nearest-required'){rgb++;check('Explicit RGB nearest',m.emissiveMap?.magFilter===T.NearestFilter&&m.emissiveMap?.minFilter===T.NearestFilter&&!m.emissiveMap?.generateMipmaps)}}});return{masks,rgb}};
const mode=async(value,row)=>{select('State preview view',value);await until(()=>value==='art'?v.stateDelivery.mode==='native-art':v.stateDelivery.mode==='physical-endpoint'&&v.deliveryEndpoint===value,row.id+' '+value)};
const missionButton=[...document.querySelectorAll('.editor-modes button')].find(b=>b.textContent.trim()==='Mission');check('Mission mode exists',missionButton);missionButton.click();await until(()=>document.querySelector('select[aria-label="Mission"]'),'mission picker');v.setTextureDisplay(true,true);
let lastRoot=null,priorReceivers=[];
for(const mission of cfg.missions){
 select('Mission',mission);
 await until(()=>v.bindings.document()?.mission?.importedFrom===mission&&document.querySelector('select[aria-label="State preview asset"]'),mission+' imported');
 const rows=cfg.rows.filter(r=>r.mission===mission);
 await until(()=>{const input=document.querySelector('select[aria-label="State preview asset"]');return rows.every(r=>[...input.options].some(o=>o.value===r.id))},mission+' catalog');
 const offered=[...document.querySelector('select[aria-label="State preview asset"]').options].map(o=>o.value);check('Mission filtering '+mission,cfg.rows.filter(r=>r.mission!==mission).every(r=>!offered.includes(r.id)));for(const id of cfg.originalEntries.filter(r=>r.mission===mission).map(r=>r.id))check('Original control preserved '+id,offered.includes(id));evidence.missions.push({mission,offered});
 for(const row of rows){
  select('State preview asset',row.id);await until(()=>v.stateDelivery.ready&&v.deliveryFamily===row.family&&document.querySelector('select[aria-label="State preview view"]'),row.id+' loaded');
  if(lastRoot){check('Previous aperture detached '+row.id,lastRoot.parent===null);lastRoot=null}
  check('Loaded mission '+row.id,v.deliveryMission===mission);check('Loaded source contract '+row.id,v.deliveryStaticContract.native.mission_data_sha256===row.mission_data_sha256);check('Endpoint model bindings '+row.id,same(v.deliveryStaticContract.families[0].physical,row.physical));
  const initialHash=await hash(v.stateDelivery.native.pixels().data);let screenshotObject;
  if(row.kind==='hole'){
   check('Catalog installed aperture '+row.id,!!v.aperturePreview);const preview=v.aperturePreview,root=preview.root;lastRoot=root;priorReceivers=preview.receivers.map(o=>({object:o,visible:o.visible}));await mode('initial',row);
   check('Original receivers hidden '+row.id,priorReceivers.every(r=>!r.object.visible));check('Duplicate delivery hidden '+row.id,!v.stateDelivery.physical.visible);const site=root.getObjectByName(row.site);check('Focused site exists '+row.id,site);screenshotObject=site;
   check('Initial endpoint visible '+row.id,visible(site.children[0])&&!visible(site.children[1]));const policy=sampling(root);check('Hole sampler contract present '+row.id,policy.masks>0&&policy.rgb>0);
   if(row.capture)await capture(row.capture+'-initial',site);
   await mode('applied',row);check('Applied endpoint visible '+row.id,!visible(site.children[0])&&visible(site.children[1]));let focusedCaps=0;root.traverse(o=>{if(!o.userData.aperture_component)return;const focused=o.userData.aperture_component===row.site;if(focused)focusedCaps++;check('Cap ownership '+row.id+' '+o.name,focused?!visible(o):visible(o))});check('Focused caps found '+row.id,focusedCaps>0);
   if(row.capture)await capture(row.capture+'-applied',site);
   await mode('art',row);check('Native restores receivers '+row.id,priorReceivers.every(r=>r.object.visible===r.visible));check('Native hides derivative '+row.id,!root.visible);
  }else{
   check('Mound has no aperture '+row.id,!v.aperturePreview);await mode('initial',row);const [initial,applied]=v.stateDelivery.physical.children;check('Initial mound visible '+row.id,initial.children.length===1&&visible(initial));screenshotObject=initial;sampling(v.stateDelivery.physical);if(row.capture)await capture(row.capture+'-initial',initial);
   await mode('applied',row);check('Mound applied state '+row.id,!visible(initial)&&visible(applied)&&applied.children.length===(row.absent?0:1));if(row.capture&&!row.absent)await capture(row.capture+'-applied',applied);if(row.absent&&row.capture)await capture(row.capture+'-absent',initial);
   await mode('art',row);check('Mound native hides physical '+row.id,!v.stateDelivery.physical.visible);
  }
  check('Mode switches preserve native pixels '+row.id,await hash(v.stateDelivery.native.pixels().data)===initialHash);
  const frame=document.querySelector('input[aria-label="State preview frame"]');check('Native frame control '+row.id,frame);frame.value=String(row.terminal_tick);frame.dispatchEvent(new Event('input',{bubbles:true}));await until(()=>v.stateDelivery.familyTick(row.family)===row.terminal_tick,row.id+' terminal');
  const reset=[...document.querySelectorAll('[aria-label="State preview"] button')].find(b=>b.textContent.trim()==='Reset');check('Reset UI '+row.id,reset);reset.click();await until(()=>v.stateDelivery.familyTick(row.family)===undefined,row.id+' reset');check('Reset restores pixels '+row.id,await hash(v.stateDelivery.native.pixels().data)===initialHash);
  evidence.controls.push({id:row.id,kind:row.kind,mission,site:row.site,applied_absent:row.absent,initial_rgba_sha256:initialHash,loaded:true,mode_and_reset_pass:true});
 }
}
// Receiver invalidation uses the normal editor transform commit, then restores it without saving.
const guard=cfg.rows.find(r=>r.kind==='hole');select('Mission',guard.mission);await until(()=>v.bindings.document()?.mission?.importedFrom===guard.mission&&[...document.querySelector('select[aria-label="State preview asset"]').options].some(o=>o.value===guard.id),'guard mission');select('State preview asset',guard.id);await until(()=>v.deliveryFamily===guard.family&&v.aperturePreview,'guard load');await mode('initial',guard);
const originals=v.aperturePreview.receivers.map(o=>o),placement=v.bindings.document().groups.find(g=>g.id===cfg.bankGroup),originalTransform=structuredClone(placement.transform),oldError=v.bindings.onError;v.bindings.onError=message=>evidence.expectedErrors.push(message);
try{v.bindings.onSelection({kind:'group',id:cfg.bankGroup},false);v.bindings.commitTransform({...originalTransform,dx:originalTransform.dx+1});await until(()=>!v.stateDelivery.ready,'edited receiver rejected');check('Edited receiver rejection is explicit',evidence.expectedErrors.some(e=>/Aperture receiver (placement changed|moved)/.test(e)),evidence.expectedErrors);check('Edited receiver originals restored',originals.every(o=>o.visible));check('Edited receiver derivative retired',!v.aperturePreview);v.bindings.commitTransform(originalTransform);await sleep(100)}finally{v.bindings.onError=oldError}
let ordinaryRgb=0;v.objectsRoot.traverse(o=>{if(!o.isMesh)return;for(const m of Array.isArray(o.material)?o.material:[o.material])if(m.userData.export_complementary_binary_ownership&&m.userData.export_rgb_sampling!=='nearest-required'&&m.emissiveMap){ordinaryRgb++;check('Unmarked wall RGB stays smooth',m.emissiveMap.magFilter===T.LinearFilter)}});check('Existing unmarked RGB tested',ordinaryRgb>0);
select('Mission','');await until(()=>!v.bindings.document()?.mission?.importedFrom&&!v.stateDelivery.ready&&!v.aperturePreview,'map-only retirement');check('All62 controls accepted',evidence.controls.length===62);check('All20 mound sites accepted',new Set(evidence.controls.filter(r=>r.kind==='mound').map(r=>r.site)).size===20);check('Absent mound tested',evidence.controls.some(r=>r.applied_absent));
return{status:'SCATTER_EXECUTED_PENDING_VISUAL_REVIEW',checks,evidence,scope:'Actual full editor catalog/UI,62focused controls, receiver restoration and source sampling. Representative original-direction context captures; no publication or gameplay claim.'};
})()
