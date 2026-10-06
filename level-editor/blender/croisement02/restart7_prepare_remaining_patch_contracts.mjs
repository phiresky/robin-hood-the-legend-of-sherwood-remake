// Private native patch previews; no physical endpoint or gameplay claim.
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import {createHash} from 'node:crypto';
import {validateNativeStatePresentation} from '../../shared/src/native-state-presentation.ts';
import {verifyNativePresentationSource} from '../../app/src/native-state-presentation.ts';
const base=resolve('level-editor/work/croisement02-refinement'),lib=resolve('level-editor/library'),stage=join(base,'restart2-state/remaining-seven-package-v2/library'),out=join(base,'restart7-source-patch-delivery/contracts-v1');
const read=async p=>JSON.parse(await readFile(p,'utf8')),sha=b=>createHash('sha256').update(b).digest('hex'),hash=x=>sha(JSON.stringify(x));
const audit=await read(join(base,'restart2-state/finite-state-coverage-reconciliation-v1.json')),catalog=await read(join(stage,'mission-states/index.json')),layers=await read(join(base,'source-states/layers.json')),level=await read(join(lib,'game-data/Data/Levels/Croisement02.rhp.json')),resources=new Map(),records=[];
await mkdir(join(out,'contracts'),{recursive:true});await mkdir(join(out,'profiles'),{recursive:true});
const resource=async(source,ext='png')=>{const bytes=await readFile(source),h=sha(bytes),path='mission-states/croisement02/resources/'+h+'.'+ext;resources.set(path,{path,sha256:h,source,bytes:bytes.length});return{path,sha256:h,...(ext==='png'?{width:bytes.readUInt32BE(16),height:bytes.readUInt32BE(20)}:{})}};
const templates=new Map();
for(const name of [...new Set(audit.remaining_uncontrolled_patch_instances.map(r=>r.mission))]){
 const entry=catalog.entries.find(e=>e.mission===name&&e.id.includes('net'))??catalog.entries.find(e=>e.mission===name);
 let c;try{c=await read(join(stage,entry.contract.path))}catch{c=await read(join(lib,entry.contract.path))}
 const native=c.native,mission=await read(join(lib,'game-data/Data/Levels',name+'.rhm.json'));
 native.background_states=[];native.patch_states=[];
 for(const [index,raw] of mission.mission_patches.entries()){
  if(!raw.element_fx.active)continue;
  const record=layers.mission_patches.find(r=>r.mission===name&&r.mission_patch_index===index);if(!record)throw Error('Missing source patch '+name+index);
  const sprite=raw.element_fx.sprite,bank=await read(resolve('datadirs/fullgame_gog_hackable/Data/Animations/Day',sprite.frame_profile_name+'.rhs.d/manifest.json')),profile=bank.profiles.find(p=>p.name===sprite.profile_name);
  if(!profile)throw Error('Missing profile');const pf=join(out,'profiles',hash(profile)+'.json');await writeFile(pf,JSON.stringify(profile,null,2)+'\n');
  const position=[sprite.position_x,sprite.position_y],center=[profile.center_x,profile.center_y];
  const frames=async phase=>Promise.all((record.states[phase]?.frames??[]).map(async f=>({...await resource(join(base,'source-states',f.image)),offset:[f.bbox[0]-position[0],f.bbox[1]-position[1]],delay:f.delay})));
  const initial=await frames('initial'),transition=await frames('transition'),final=await frames('final'),all=[...initial,...transition,...final];
  const x=Math.min(...all.map(f=>position[0]+f.offset[0])),y=Math.min(...all.map(f=>position[1]+f.offset[1])),right=Math.max(...all.map(f=>position[0]+f.offset[0]+f.width)),bottom=Math.max(...all.map(f=>position[1]+f.offset[1]+f.height));
  native.patch_states.push({id:record.id,source:{kind:'mission-patch',index,sha256:hash(raw)},profile:{...await resource(pf,'json'),name:profile.name,center},integrate_in_background:raw.integrate_in_background,...(raw.integrate_in_background?{activation:'initial-only',restore_bounds:[x,y,right-x,bottom-y]}:{}),elevation:sprite.elevation,layer:sprite.elevation===0?'background':'ordered',display_position:position,sort_position:[position[0]+center[0],position[1]+center[1]],display_order:position[1]+center[1]+sprite.elevation,creation_order:level.patches.length+level.animations.length+mission.targets.length+index,polyline:raw.element_fx.display_polyline,definitive:raw.definitive,initial,transition,final,initial_loop:true,final_loop:true});
 }
 templates.set(name,{native,mission});
}
for(const row of audit.remaining_uncontrolled_patch_instances){
 const {native:template,mission}=templates.get(row.mission),native=structuredClone(template),focus=native.patch_states.find(p=>p.id===row.id);
 if(focus.integrate_in_background)focus.activation='phases';
 const contract={version:1,kind:'native-patch',native,focus_patch_id:focus.id,scope:'One controlled native patch; all surrounding patches initial. No physical endpoint, actor simulation or gameplay execution.'};
 try{validateNativeStatePresentation(native);await verifyNativePresentationSource(native,{name:row.mission,data:mission,level,camera:{kind:'oblique-orthographic',elevation_deg:35}});const file='contracts/'+row.id+'.json',bytes=JSON.stringify(contract,null,2)+'\n';await writeFile(join(out,file),bytes);records.push({...row,contract:file,sha256:sha(bytes),status:'SOURCE_BINDINGS_PASS',terminal_tick:Math.max(1,focus.transition.reduce((n,f)=>n+f.delay+1,0)-1),initial_frames:focus.initial.length,transition_frames:focus.transition.length,final_frames:focus.final.length,integrating:focus.integrate_in_background,definitive:focus.definitive});}catch(error){records.push({...row,status:'HOLD',error:String(error)})}
}
await writeFile(join(out,'manifest.json'),JSON.stringify({status:'PRIVATE_SOURCE_CONTRACT_PREPARATION; runtime parity and source-only UI proposal pending',denominator:{mission_patch_instances:129,previously_controlled:47,remaining:82},records,resources:[...resources.values()],scope:'No live catalog or runtime modifications; native-patch wrapper is a private proposal only.'},null,2)+'\n');console.log(JSON.stringify({pass:records.filter(r=>r.status==='SOURCE_BINDINGS_PASS').length,total:records.length,holds:records.filter(r=>r.error).map(r=>({id:r.id,error:r.error}))}));
