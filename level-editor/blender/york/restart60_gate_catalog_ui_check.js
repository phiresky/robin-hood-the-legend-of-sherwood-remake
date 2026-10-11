(async()=>{
 const cfg=window.yorkGateUiConfig,v=window.reviewViewport,T=window.reviewThree;
 const evidence=window.yorkGateUiEvidence={checks:[],captures:[],poses:[],http:[]};
 const check=(name,pass,detail)=>{evidence.checks.push({name,pass:!!pass,detail});if(!pass)throw Error(name+': '+JSON.stringify(detail));};
 const sleep=ms=>new Promise(r=>setTimeout(r,ms)),until=async(fn,label)=>{for(let i=0;i<2400;i++){if(await fn())return;await sleep(100)}throw Error('Timeout '+label)};
 const capture=async name=>{window.scatterStage={name,time:Date.now()};await until(()=>window.scatterCaptures?.[name],name);evidence.captures.push(name)};
 const hash=async bytes=>[...new Uint8Array(await crypto.subtle.digest('SHA-256',bytes))].map(x=>x.toString(16).padStart(2,'0')).join('');
 const read=async(path,sha)=>{const r=await fetch(path,{cache:'no-store'});check('HTTP readable '+path,r.ok);const b=await r.arrayBuffer();check('HTTP pin '+path,await hash(b)===sha);evidence.http.push({path,sha256:sha});return JSON.parse(new TextDecoder().decode(b))};
 const contract=await read('/library/'+cfg.entry.contract.path,cfg.entry.contract.sha256);
 const doc=v.bindings.document();check('York normal map loaded',doc?.groups.some(g=>g.id==='york-castle-west-gatehouse'));
 const camera=index=>{const row=cfg.views[index],m=row.camera_matrix_world,cv=p=>new T.Vector3(p[0],p[2],-p[1]),pos=cv([m[0][3],m[1][3],m[2][3]]),back=cv([m[0][2],m[1][2],m[2][2]]);v.setPerspective(0);v.applyState(v.lookState(pos,pos.clone().addScaledVector(back,-10000),row.ortho_scale/2));};
 [...document.querySelectorAll('button')].find(b=>b.textContent==='Mission').click();
 await until(()=>document.querySelector('select[aria-label="Mission"]'),'mission tab');
 const mission=document.querySelector('select[aria-label="Mission"]');
 if(mission.value!==cfg.entry.mission){mission.value=cfg.entry.mission;mission.dispatchEvent(new Event('change',{bubbles:true}));}
 await until(()=>v.entities?.missionName===cfg.entry.mission&&!document.querySelector('.map-load-dialog'),'reactive mission publication');
 await until(()=>document.querySelector('select[aria-label="State preview asset"]')?.querySelector('option[value="'+cfg.entry.id+'"]'),'catalog option');
 const asset=document.querySelector('select[aria-label="State preview asset"]');
 if(asset.value!==cfg.entry.id){asset.value=cfg.entry.id;asset.dispatchEvent(new Event('change',{bubbles:true}));}
 await until(()=>v.deliveryMission===cfg.entry.mission&&v.stateDelivery.hasFamily('york-portcullis')&&!document.querySelector('[aria-label="State preview"] [role="status"]'),'reactive state selection publication');
 const panel=()=>document.querySelector('[aria-label="State preview"]');
 check('Catalog selected approved motion contract',v.stateDelivery.contract.families[0].physical.initial[0].model_sha256===contract.families[0].physical.initial[0].model_sha256&&v.stateDelivery.contract.families[0].physical.motion.source_motion_sha256===contract.families[0].physical.motion.source_motion_sha256);
 async function mode(value,expected){const select=panel().querySelector('select[aria-label="State preview view"]');check('UI mode available '+value,[...select.options].some(o=>o.value===value));select.value=value;select.dispatchEvent(new Event('change',{bubbles:true}));await until(()=>v.stateDelivery.mode===expected,'mode '+value);}
 function button(text){const b=[...panel().querySelectorAll('button')].find(b=>b.textContent===text);check('UI button '+text,!!b);b.click();}
 await mode('motion','physical-motion');await until(()=>v.stateDelivery.physical.visible,'physical roots visible');camera(0);await capture('york-gate-ui-initial-source');
 const [covered,raised]=v.stateDelivery.physical.children;
 check('Two loaded endpoint roots',covered?.children.length===1&&raised?.children.length===1);button('Play');
 await until(()=>v.deliveredStateStatus('york-portcullis').tick>=10,'actual viewport clock advanced');
 const played=v.deliveredStateStatus('york-portcullis');check('Physical motion plays from normal UI',played.playing&&covered.position.y>0,{tick:played.tick,y:covered.position.y});
 button('Pause');await until(()=>!v.deliveredStateStatus('york-portcullis').playing,'pause settled');
 for(const tick of [44,88,89]){
  const slider=panel().querySelector('input[aria-label="State preview frame"]');slider.value=String(tick);slider.dispatchEvent(new Event('input',{bubbles:true}));await until(()=>v.deliveredStateStatus('york-portcullis').tick===tick,'slider tick '+tick);
  const applied=tick>=89,offset=applied?[0,0,0]:contract.families[0].physical.motion.translation_frames[Math.floor(tick/2)];
  check('UI exact pose '+tick,JSON.stringify(covered.position.toArray())===JSON.stringify(offset)&&covered.visible===!applied&&raised.visible===applied);
  check('UI raised no double lift '+tick,raised.position.y===0);evidence.poses.push({tick,offset,covered:covered.visible,raised:raised.visible});
  camera(0);await capture('york-gate-ui-tick-'+tick+'-source');if(tick===89){camera(2);await capture('york-gate-ui-applied-oblique');}
 }
 button('Reset');await until(()=>v.deliveredStateStatus('york-portcullis').tick===undefined,'reset settled');check('UI reset restores covered pose',covered.visible&&!raised.visible&&covered.position.y===0);camera(0);await capture('york-gate-ui-reset-source');
 const box=new T.Box3().setFromObject(v.stateDelivery.physical).expandByScalar(50);evidence.nearbyStaticGroups=[...v.groupViews.entries()].filter(([id,view])=>box.intersectsBox(new T.Box3().setFromObject(view.wrapper))).map(([id])=>id);
 v.stateDelivery.physical.visible=false;await capture('york-gate-ui-static-only-source');v.stateDelivery.physical.visible=true;
 await mode('art','native-art');check('Native mode hides physical preview',!v.stateDelivery.physical.visible);await capture('york-gate-ui-native-initial');
 mission.value='';mission.dispatchEvent(new Event('change',{bubbles:true}));await until(()=>!v.stateDelivery.ready&&v.stateDelivery.physical.children.length===0&&!v.entities,'Map-only retirement');check('Map-only removes state roots',!v.stateDelivery.physical.visible);await capture('york-gate-ui-map-only');
 return{status:'EXECUTED_NORMAL_EDITOR_PENDING_VISUAL_REVIEW',entry_id:cfg.entry.id,contract_sha256:cfg.entry.contract.sha256,editor_checks:{map_load:true,catalog_selection:true,physical_motion_play:true,seek_terminal:true,reset:true,native_mode:true,map_only_cleanup:true},...evidence};
})()
