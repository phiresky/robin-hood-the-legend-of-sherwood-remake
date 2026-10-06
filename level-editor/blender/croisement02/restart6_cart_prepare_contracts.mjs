import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import {createHash} from 'node:crypto';
import {validateStateDelivery} from '../../shared/src/state-delivery.ts';
import {verifyNativePresentationSource} from '../../app/src/native-state-presentation.ts';
import {placementHeight} from '../../app/src/entity-projection.ts';
const b=resolve('level-editor/work/croisement02-refinement'),lib=resolve('level-editor/library'),out=join(b,'restart2-state/cart-contract-preparation-v2');
const read=async p=>JSON.parse(await readFile(p,'utf8')),sha=b=>createHash('sha256').update(b).digest('hex'),hash=x=>sha(JSON.stringify(x));
const instances=(await read(join(b,'state-target-evidence/manifest.json'))).instances,initial=(await read(join(b,'restart2-state/cart-initial-action-audit-v1/report.json'))).records,ends=await read(join(b,'restart2-state/remaining-local-origins-v1/manifest.json')),level=await read(join(lib,'game-data/Data/Levels/Croisement02.rhp.json')),resources=new Map(),records=[];
await mkdir(join(out,'contracts'),{recursive:true});
const resource=async(file,extension)=>{const bytes=await readFile(file),s=sha(bytes),path='mission-states/croisement02/resources/'+s+'.'+extension;resources.set(path,{path,sha256:s,source:file,bytes:bytes.length});return {path,sha256:s,...(extension==='png'?{width:bytes.readUInt32BE(16),height:bytes.readUInt32BE(20)}:{})}};
const image=async f=>({...await resource(f.image,'png'),offset:f.offset,delay:f.delay});
const model=async (id,file,expected,position)=>{if(sha(await readFile(file))!==expected)throw Error('Model hash changed '+id);const r=await resource(file,'glb');return{id,role:'objects',model:r.path,model_sha256:r.sha256,resources:[],position}};
for(const [family,missionName] of [['south-cart','Emb05_FoB_MP'],['south-cart','Tac19_FoB_EC'],['north-cart','Emb09_FoB_JMS']]){
 const mission=await read(join(lib,'game-data/Data/Levels',missionName+'.rhm.json')),evidence=await read(join(b,'state-target-evidence',family,'manifest.json'));
 const template=missionName==='Tac19_FoB_EC'?'net-03-empty':'net-01-empty';
 const contract=await read(join(lib,'mission-states/croisement02/contracts',missionName+'-'+template+'.json'));const native=contract.native;
 native.background_states=[];native.patch_states=(native.patch_states??[]).filter(s=>s.integrate_in_background);
 const ids=[],timing=[];
 for(const part of evidence.parts){
  const instance=instances.find(i=>i.mission===missionName&&i.profile_id===part.profile_id);if(!instance)throw Error('Missing source target');
  const target=mission.targets[instance.target_index],start=initial.find(i=>i.profile_id===part.profile_id&&i.action===target.action);if(!start)throw Error('Unsurveyed initial action');
  const z=target.position_z>=0?target.position_z:target.obstacle_index===65535?0:placementHeight(target.position_x,target.position_y,level.sight_obstacles[target.obstacle_index]);
  const stationary=await image(start.initial_frame),frames=await Promise.all(part.frames.map(image));
  if(part.start_tick)frames.unshift({...stationary,delay:part.start_tick-1});
  const id='target'+instance.target_index;ids.push(id);native.elements=native.elements.filter(e=>e.id!==id);native.elements.push({id,source:{kind:'mission-target',index:instance.target_index,sha256:hash(target)},active:false,loop:false,frames,initial_frame:stationary,display_position:[target.position_x,target.position_y],sort_position:[target.action_position_x,target.action_position_y],display_order:target.position_y+z,creation_order:level.patches.length+level.animations.length+instance.target_index,polyline:target.polyline});
  timing.push({id,profile:part.profile_id,start_tick:part.start_tick,initial_action:target.action,hold_uses_exact_initial_frame:part.start_tick>0,source_row_delay_unchanged:true,terminal_tick:part.terminal_frame_reached_tick});
 }
 const selected=ends.records.filter(r=>r.family===family&&r.id.includes('terminal'));
 const applied=await Promise.all(selected.map(r=>model(r.id,join(b,'restart2-state/remaining-local-origins-v1',r.glb),r.glb_sha256,r.position)));
 const physicalInitial=family==='north-cart'?{kind:'absent'}:[await model('croisement02-south-cart-initial-physical',join(b,'restart4-south-cart-export/private-v3/initial-wagon.glb'),'c571a249155dd0fcdfeba6d01b6b49ce8b0d27b8941433cc6042b94c81174272',ends.anchors['south-cart'])];
 contract.families=[{id:family,element_ids:ids,background_ids:[],body_terminal_tick:evidence.terminal_geometry_reached_tick,physical:{initial:physicalInitial,applied}}];
 try{validateStateDelivery(contract);await verifyNativePresentationSource(native,{name:missionName,data:mission,level,camera:{kind:'oblique-orthographic',elevation_deg:35}});const file=join('contracts',missionName+'-'+family+'.json'),bytes=JSON.stringify(contract,null,2)+'\n';await writeFile(join(out,file),bytes);records.push({mission:missionName,family,contract:file,sha256:sha(bytes),status:'SOURCE_BINDINGS_PASS; independent native timing/composition and Editor proof pending',timing,script_evidence:evidence.script_evidence});}catch(e){records.push({mission:missionName,family,status:'HOLD',error:String(e),timing});}
}
await writeFile(join(out,'manifest.json'),JSON.stringify({scope:'Private controlled cart component sequences. Delayed components retain their exact initial frame until the surveyed start tick. Player-dismissed popup duration, actors, separate fence clearing and gameplay scripts are not executed. North pre-trigger target is absent; first-visible physical reconstruction is not mislabeled initial.',records,resources:[...resources.values()]},null,2)+'\n');console.log(records.map(r=>({mission:r.mission,status:r.status,error:r.error})));
