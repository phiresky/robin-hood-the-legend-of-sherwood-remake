// Private source-art control packets; physical geometry and gameplay remain separate.
import {readFile, writeFile, mkdir} from 'node:fs/promises';
import {resolve, join} from 'node:path';
import {createHash} from 'node:crypto';
import {validateNativeStatePresentation} from '../../shared/src/native-state-presentation.ts';
import {verifyNativePresentationSource} from '../../app/src/native-state-presentation.ts';
import {validateNativePatchPreview} from '../../shared/src/state-delivery.ts';

const root=resolve('.'), work=join(root,'level-editor/work/active/croisement01');
const lib=join(root,'level-editor/library'), out=join(work,'restart2/native-controls-private-oct11-v2');
await mkdir(out); await mkdir(join(out,'contracts')); await mkdir(join(out,'profiles'));
const read=async p=>JSON.parse(await readFile(p,'utf8'));
const sha=b=>createHash('sha256').update(b).digest('hex'), hash=v=>sha(JSON.stringify(v));
const resources=new Map(), records=[], entries=[];
async function resource(source, ext='png') {
  const bytes=await readFile(source), digest=sha(bytes);
  const path=`mission-states/croisement01/resources/${digest}.${ext}`;
  resources.set(path,{path,sha256:digest,source,bytes:bytes.length});
  return {path,sha256:digest,...(ext==='png'?{width:bytes.readUInt32BE(16),height:bytes.readUInt32BE(20)}:{})};
}
const levelPath=join(lib,'game-data/Data/Levels/Croisement01.rhp.json');
const level=await read(levelPath), levelPin=await resource(levelPath,'json');
const layers=await read(join(work,'source-states/layers.json'));
const animation=await read(join(work,'animation-references/manifest.json'));
const background=await resource(join(work,'baseline/covered.png'));
const bankCache=new Map();
async function profile(bankName, name) {
  if(!bankCache.has(bankName)) bankCache.set(bankName,await read(join(root,'datadirs/fullgame_gog_hackable/Data/Animations/Day',bankName+'.rhs.d/manifest.json')));
  const found=bankCache.get(bankName).profiles.find(p=>p.name===name);
  if(!found) throw Error('Missing sprite profile '+name);
  return found;
}
const globalElements=[];
for (const [index,raw] of level.animations.entries()) {
  const record=animation.animations.find(a=>a.index===index), sprite=raw.sprite;
  if(!record || record.profile!==sprite.profile_name) throw Error('Animation binding mismatch');
  const p=await profile(sprite.frame_profile_name,sprite.profile_name);
  const frames=[];
  for(const f of record.frames) {
    const pin=await resource(f.image);
    if(sha(await readFile(f.source))!==f.sha256) throw Error('Animation source frame changed');
    frames.push({...pin,offset:[f.bbox[0]-sprite.position_x,f.bbox[1]-sprite.position_y],delay:f.delay});
  }
  globalElements.push({id:`animation-${index}`,source:{kind:'map-animation',index,sha256:hash(raw)},active:raw.active,frames,loop:true,
    display_position:[sprite.position_x,sprite.position_y],sort_position:[sprite.position_x+p.center_x,sprite.position_y+p.center_y],
    display_order:sprite.position_y+p.center_y+sprite.elevation,creation_order:level.patches.length+index,polyline:raw.display_polyline});
}
for(const missionName of [...new Set(layers.mission_patches.map(p=>p.mission))].sort()) {
  const missionPath=join(lib,'game-data/Data/Levels',missionName+'.rhm.json'), mission=await read(missionPath), missionPin=await resource(missionPath,'json');
  const native={version:1,mission:missionName,mission_data_sha256:hash(mission),level_data_sha256:hash(level),camera_elevation_deg:35,
    scope:'map-art-and-listed-effects',background,origin:[0,0],elements:structuredClone(globalElements),patch_states:[]};
  for(const [index,t] of mission.targets.entries()) {
    if(!t.polyline?.length) continue;
    native.elements.push({id:`target-boundary-${index}`,source:{kind:'mission-target',index,sha256:hash(t)},active:true,frames:[],loop:false,
      display_position:[t.position_x,t.position_y],sort_position:[t.action_position_x,t.action_position_y],
      display_order:t.action_position_y+t.position_z,creation_order:level.patches.length+level.animations.length+index,polyline:t.polyline});
  }
  for(const [index,raw] of mission.mission_patches.entries()) {
    const record=layers.mission_patches.find(r=>r.mission===missionName&&r.mission_patch_index===index);
    if(!record) throw Error('Missing decoded patch '+missionName+':'+index);
    if(!raw.element_fx.active || !raw.transition_animation_valid) {
      records.push({mission:missionName,id:record.id,index,status:'HOLD_NONTRANSITION_OR_INACTIVE',reason:'Requires a separately scoped control representation.'}); continue;
    }
    const sprite=raw.element_fx.sprite,p=await profile(sprite.frame_profile_name,sprite.profile_name);
    const profilePath=join(out,'profiles',hash(p)+'.json'); await writeFile(profilePath,JSON.stringify(p)+'\n');
    const position=[sprite.position_x,sprite.position_y],center=[p.center_x,p.center_y];
    const frames=async phase=>Promise.all((record.states[phase]?.frames??[]).map(async f=>({...await resource(join(work,'source-states',f.image)),offset:[f.bbox[0]-position[0],f.bbox[1]-position[1]],delay:f.delay})));
    const initial=await frames('initial'),transition=await frames('transition'),final=await frames('final'),all=[...initial,...transition,...final];
    const x=Math.min(...all.map(f=>position[0]+f.offset[0])),y=Math.min(...all.map(f=>position[1]+f.offset[1]));
    const right=Math.max(...all.map(f=>position[0]+f.offset[0]+f.width)),bottom=Math.max(...all.map(f=>position[1]+f.offset[1]+f.height));
    native.patch_states.push({id:record.id,source:{kind:'mission-patch',index,sha256:hash(raw)},profile:{...await resource(profilePath,'json'),name:p.name,center},
      integrate_in_background:raw.integrate_in_background,...(raw.integrate_in_background?{activation:'initial-only',restore_bounds:[x,y,right-x,bottom-y]}:{}),
      elevation:sprite.elevation,layer:sprite.elevation===0?'background':'ordered',display_position:position,sort_position:[position[0]+center[0],position[1]+center[1]],
      display_order:position[1]+center[1]+sprite.elevation,creation_order:level.patches.length+level.animations.length+mission.targets.length+index,
      polyline:raw.element_fx.display_polyline,definitive:raw.definitive,initial,transition,final,initial_loop:true,final_loop:true});
  }
  for(const focus of native.patch_states) {
    const selected=structuredClone(native),own=selected.patch_states.find(p=>p.id===focus.id);
    if(own.integrate_in_background) own.activation='phases';
    const contract={version:1,kind:'native-patch',native:selected,focus_patch_id:focus.id,
      scope:'One controlled source-art patch; surrounding initial patches and all native animations. Mission target boundaries only: target artwork, actors, physical endpoints and gameplay scripts excluded.'};
    try {
      validateNativeStatePresentation(selected);validateNativePatchPreview(contract);
      await verifyNativePresentationSource(selected,{name:missionName,data:mission,level,camera:{kind:'oblique-orthographic',elevation_deg:35}});
      const file=join(out,'contracts',focus.id+'.json'),bytes=JSON.stringify(contract)+'\n';await writeFile(file,bytes);
      const path='mission-states/croisement01/contracts/'+focus.id+'.json',digest=sha(bytes);
      resources.set(path,{path,source:file,sha256:digest,bytes:Buffer.byteLength(bytes)});
      records.push({id:focus.id,mission:missionName,index:focus.source.index,status:'SOURCE_BINDINGS_PASS',contract:file,sha256:digest,
        terminal_tick:Math.max(1,focus.transition.reduce((n,f)=>n+f.delay+1,0)-1)});
      entries.push({id:'croisement01-source-'+focus.id,name:focus.profile.name+' / '+focus.source.index,kind:'native-patch',map:'Croisement01',mission:missionName,
        contract:{path,sha256:digest},mission_data:missionPin,level_data:levelPin});
    } catch(error) {records.push({id:focus.id,mission:missionName,index:focus.source.index,status:'HOLD',error:String(error)});}
  }
}
await writeFile(join(out,'entries.json'),JSON.stringify({version:1,entries},null,2)+'\n');
await writeFile(join(out,'manifest.json'),JSON.stringify({status:'PRIVATE_PREPARATION_NOT_RUNTIME_VERIFIED',records,resources:[...resources.values()],
  native_animation_count:globalElements.length,source_patch_count:layers.mission_patches.length,
  limitations:['No catalog publication.','No geometry, appearance or runtime completion claim.','Target artwork is absent; target entries preserve ordering boundaries only.','Source clock, pixel composition and browser execution still require independent checks.']},null,2)+'\n');
console.log(JSON.stringify({pass:records.filter(r=>r.status==='SOURCE_BINDINGS_PASS').length,total:records.length,holds:records.filter(r=>r.status!=='SOURCE_BINDINGS_PASS')}));
