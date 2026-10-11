/** Production StateDelivery fixture. Caller owns the context renderer and capture IO. */
import {StateDelivery} from '../../app/src/state-delivery.ts';
import {NativeStatePresentation} from '../../app/src/native-state-presentation.ts';
const check=(value,message)=>{if(!value)throw Error(message);};
const equal=(a,b,message)=>check(JSON.stringify(a)===JSON.stringify(b),message);
async function digest(bytes){return Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',bytes)),b=>b.toString(16).padStart(2,'0')).join('');}

/** resolveURL maps an exact absolute pinned filesystem path to its HTTP route. */
export async function loadProductionGateInputs(request,resolveURL){
  check(typeof resolveURL==='function','Explicit HTTP path mapping required');
  const cache=new Map();
  async function bytes(pin){
    const key=pin.path+':'+pin.sha256;
    if(!cache.has(key))cache.set(key,(async()=>{const response=await fetch(resolveURL(pin.path));check(response.ok,'HTTP resource failed: '+pin.path);const data=await response.arrayBuffer();check(await digest(data)===pin.sha256,'HTTP source hash differs: '+pin.path);return data;})());
    return cache.get(key);
  }
  const json=async pin=>JSON.parse(new TextDecoder().decode(await bytes(pin)));
  const contract=await json(request.contract),mission=await json(request.source.mission),level=await json(request.source.level);
  const paths=new Map();
  for(const resource of request.resources)paths.set(resource.path,{path:request.library+'/'+resource.path,sha256:resource.sha256});
  for(const family of contract.families)for(const endpoint of [family.physical.initial,family.physical.applied])for(const binding of endpoint){paths.set(binding.model,{path:request.library+'/'+binding.model,sha256:binding.model_sha256});for(const resource of binding.resources)paths.set(resource.path,{path:request.library+'/'+resource.path,sha256:resource.sha256});}
  const missing=path=>new DOMException('Not in pinned gate library: '+path,'NotFoundError');
  function directory(prefix=''){
    return{kind:'directory',name:prefix.split('/').filter(Boolean).at(-1)??'gate',
      async getDirectoryHandle(name){const next=prefix+name+'/';if(![...paths.keys()].some(p=>p.startsWith(next)))throw missing(next);return directory(next);},
      async getFileHandle(name){const relative=prefix+name,pin=paths.get(relative);if(!pin)throw missing(relative);return{kind:'file',name,async getFile(){return new File([await bytes(pin)],name);}};}
    };
  }
  return{contract,source:{name:request.source.name,data:mission,level,camera:{kind:'oblique-orthographic',elevation_deg:35}},library:directory(),read:async resource=>{const pin=paths.get(resource.path);check(pin?.sha256===resource.sha256,'Unknown native resource');return new Uint8Array(await bytes(pin));},verifiedResources:()=>[...cache.keys()]};
}

/**
 * capture receives {label,view,phase,pose,tick,roots,delivery}. Add roots.physical
 * to the scene once, with no additional placement; model origins are already
 * applied by StateDelivery. Context gatehouse must already be world-positioned.
 * Saved views use world Z-up; convert camera axes once, without subtracting pivot.
 */
export async function runProductionGateBrowserFixture({request,resolveURL,gatehouse,contextViews,capture}){
  check(gatehouse?.isObject3D && gatehouse.visible,'Visible positioned gatehouse required');
  check(contextViews?.length===8 && contextViews.every((v,i)=>v.index===i),'Eight ordered cameras required');
  check(typeof capture==='function','Capture callback required');
  const input=await loadProductionGateInputs(request,resolveURL),delivery=new StateDelivery(),family=input.contract.families[0];
  check(input.contract.families.length===1 && family.id==='york-portcullis','Wrong physical family');
  check(family.body_terminal_tick===89 && family.physical.motion.translation_frames.length===45,'Wrong reviewed timeline');
  const gatePosition=gatehouse.position.toArray(),gateQuaternion=gatehouse.quaternion.toArray(),gateScale=gatehouse.scale.toArray();
  const images=[],checks=[],poses=new Set(),initialControl=new NativeStatePresentation();
  try{
    check(await delivery.set(input.contract,input.source,input.library,input.read),'Production set did not adopt');
    check(await initialControl.set(input.contract.native,input.source,input.read),'Initial-state control failed');
    initialControl.setPlaying(true);
    delivery.selectMode('physical-motion');
    const [covered,raised]=delivery.physical.children;
    check(covered?.children.length===1 && raised?.children.length===1,'Wrong loaded endpoint structure');
    for(const group of [covered,raised]){let meshes=0;group.traverse(o=>{if(o.isMesh)meshes++;});check(meshes>0,'Real loaded endpoint meshes required');}
    const local=[covered.children[0],raised.children[0]],before=local.map(o=>({position:o.position.toArray(),quaternion:o.quaternion.toArray(),scale:o.scale.toArray()}));
    const roots={physical:delivery.physical,covered,raised,gatehouse};
    function verify(tick){
      const applied=tick!==undefined && tick>=89,pose=tick===undefined?0:Math.min(44,Math.floor(tick/2));
      equal(covered.position.toArray(),applied?[0,0,0]:family.physical.motion.translation_frames[pose],'Covered reviewed offset differs');
      equal(raised.position.toArray(),[0,0,0],'Raised endpoint double-lifted');
      check(covered.visible===!applied && raised.visible===applied,'Endpoint visibility differs');
      local.forEach((o,i)=>{equal(o.position.toArray(),before[i].position,'Placed model origin changed');equal(o.quaternion.toArray(),before[i].quaternion,'Model orientation changed');equal(o.scale.toArray(),before[i].scale,'Model scale changed');equal(o.position.toArray(),family.physical.initial[0].position,'Model placement differs');});
      equal(gatehouse.position.toArray(),gatePosition,'Gatehouse moved');equal(gatehouse.quaternion.toArray(),gateQuaternion,'Gatehouse rotated');equal(gatehouse.scale.toArray(),gateScale,'Gatehouse scaled');
      return pose;
    }
    async function capturePair(label,phase,pose,tick){for(const index of [0,2]){images.push(await capture({label:label+'-view-'+index,phase,pose,tick,view:contextViews[index],roots,delivery}));verify(tick);}}
    verify(undefined);await capturePair('initial','initial',0,undefined);
    delivery.activate(family.id);delivery.setPlaying(true);
    for(let tick=0;tick<92;tick++){
      check(delivery.familyTick(family.id)===tick,'Native clock drift');const pose=verify(tick);poses.add(pose);
      checks.push({tick,pose,endpoint:delivery.motionEndpoint,translation:covered.position.toArray()});
      if([0,44,70,72,76,88,89].includes(tick))await capturePair('tick-'+tick,tick>=89?'applied':'forward',pose,tick);
      delivery.advance(1/25);initialControl.advance(1/25);
    }
    check(poses.size===45,'Not every reviewed pose was sampled');
    delivery.setPlaying(false);delivery.reset(family.id);check(delivery.familyTick(family.id)===undefined,'Reset retained clock');verify(undefined);
    equal(await digest(delivery.native.pixels().data.slice().buffer),await digest(initialControl.pixels().data.slice().buffer),'Native initial artwork not restored at current ambient clock');
    await capturePair('reset','initial',0,undefined);
    delivery.selectMode('native-art');check(!delivery.physical.visible,'Native mode retains physical roots');
    delivery.selectMode('physical-endpoint');delivery.selectEndpoint(family.id,'applied');equal(raised.position.toArray(),[0,0,0],'Endpoint mode doubles lift');
    delivery.dispose();check(!delivery.ready && !delivery.native.ready && delivery.physical.children.length===0,'Disposal retained roots or native state');
    equal(gatehouse.position.toArray(),gatePosition,'Disposal moved gatehouse');
    return{status:'PASS_PRODUCTION_STATE_DELIVERY_GATE',checks,images,verified_resources:input.verifiedResources(),all_45_poses:true,native_reset:true,disposed:true,scope:'Actual StateDelivery/default SceneAssetLoader and production native timeline. Context rendering supplied by caller. Does not prove full-editor UI catalog integration, engine authoritative snapshot transport, doors, collision or audio. Frame36 remains approved inference.'};
  }finally{delivery.dispose();initialControl.dispose();}
}
