/** Prepare source-bound mission variants without installing or synthesizing assets. */
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import {createHash} from 'node:crypto';
import {validateStateDelivery} from '../../shared/src/state-delivery.ts';
import {verifyNativePresentationSource} from '../../app/src/native-state-presentation.ts';
import {placementHeight} from '../../app/src/entity-projection.ts';
const root=resolve('level-editor'),out=join(root,'work/croisement02-refinement'),dest=join(out,'restart2-state/trap-mission-variants-v1');
const read=async p=>JSON.parse(await readFile(p,'utf8')),sha=b=>createHash('sha256').update(b).digest('hex'),hash=x=>sha(JSON.stringify(x));
const level=await read(join(root,'library/game-data/Data/Levels/Croisement02.rhp.json')),targets=await read(join(out,'state-target-evidence/manifest.json')),layers=await read(join(out,'source-states/layers.json'));
await mkdir(dest,{recursive:true});const records=[];
for(const family of ['log-trap','rock-trap']){
 const evidence=await read(join(out,'state-target-evidence',family,'manifest.json')),base=await read(join(root,'library/mission-states/croisement02/contracts',family+'.json'));
 for(const binding of evidence.background_bindings){
  if(binding.mission==='Emb05_FoB_MP')continue;
  const mission=await read(join(root,'library/game-data/Data/Levels',binding.mission+'.rhm.json')),matches=new Map(),duplicates=[];
  for(const part of evidence.parts){
   const candidates=targets.instances.filter(t=>t.mission===binding.mission&&t.profile_id===part.profile_id);
   if(!candidates.length)throw Error('Missing mission profile '+binding.mission+' '+part.profile_id);
   matches.set(candidates[0].target_index,{part,template:base.native.elements.find(e=>e.id==='target'+part.target_index)});
   duplicates.push(...candidates.slice(1).map(c=>c.target_index));
  }
  const contract=structuredClone(base),native=contract.native;
  native.mission=binding.mission;native.mission_data_sha256=hash(mission);
  native.elements=native.elements.filter(e=>e.source.kind==='map-animation');
  for(let i=0;i<mission.targets.length;i++){
   const target=mission.targets[i],match=matches.get(i);if(!match&&!target.polyline.length)continue;
   const z=target.position_z>=0?target.position_z:target.obstacle_index===65535?0:placementHeight(target.position_x,target.position_y,level.sight_obstacles[target.obstacle_index]);
   native.elements.push({...(match?structuredClone(match.template):{frames:[],loop:false}),id:'target'+i,source:{kind:'mission-target',index:i,sha256:hash(target)},active:!match,display_position:[target.position_x,target.position_y],sort_position:[target.action_position_x,target.action_position_y],display_order:target.position_y+z,creation_order:15+i,polyline:target.polyline});
  }
  const patch=layers.mission_patches.find(p=>p.id===binding.id),source=mission.mission_patches[patch.mission_patch_index],position=[source.element_fx.sprite.position_x,source.element_fx.sprite.position_y];
  const frames=async stage=>Promise.all((patch.states[stage]?.frames??[]).map(async f=>{const bytes=await readFile(join(out,'source-states',f.image)),digest=sha(bytes),path='mission-states/croisement02/resources/'+digest+'.png';if(sha(await readFile(join(root,'library',path)))!==digest)throw Error('Uninstalled source '+path);return{path,sha256:digest,width:bytes.readUInt32BE(16),height:bytes.readUInt32BE(20),offset:[f.bbox[0]-position[0],f.bbox[1]-position[1]],delay:f.delay}}));
  const initial=await frames('initial'),transition=await frames('transition'),final=await frames('final'),all=[...initial,...transition,...final],x=Math.min(...all.map(f=>position[0]+f.offset[0])),y=Math.min(...all.map(f=>position[1]+f.offset[1])),right=Math.max(...all.map(f=>position[0]+f.offset[0]+f.width)),bottom=Math.max(...all.map(f=>position[1]+f.offset[1]+f.height));
  native.background_states=[{...native.background_states[0],source:{kind:'mission-patch',index:patch.mission_patch_index,sha256:hash(source)},display_position:position,restore_bounds:[x,y,right-x,bottom-y],definitive:source.definitive,initial,transition,final}];
  contract.families[0].element_ids=[...matches.keys()].map(i=>'target'+i);
  validateStateDelivery(contract);try{await verifyNativePresentationSource(native,{name:binding.mission,data:mission,level,camera:{kind:'oblique-orthographic',elevation_deg:35}})}catch(error){records.push({mission:binding.mission,family,status:'HOLD',error:String(error)});continue;}
  const name=binding.mission+'-'+family+'.json',bytes=JSON.stringify(contract,null,2)+'\n';await writeFile(join(dest,name),bytes);
  records.push({mission:binding.mission,family,contract:name,sha256:sha(bytes),target_indices:[...matches.keys()],duplicate_profile_targets_not_activated:duplicates,scope:'Controlled selected family preview; independent approved physical endpoint hypothesis reused. No mission script execution or duplicate-target activation inferred.'});
 }
}
await writeFile(join(dest,'manifest.json'),JSON.stringify({status:'PRIVATE SOURCE AUDIT; individually recorded holds preserved; browser/context review pending',records},null,2)+'\n');console.log(records.length+' source-bound variants; no installation');
