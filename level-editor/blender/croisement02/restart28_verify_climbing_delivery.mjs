import assert from 'node:assert/strict';
import {readFile,writeFile} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import {createHash} from 'node:crypto';
import {validateStateDelivery} from '../../shared/src/state-delivery.ts';
import {verifyNativePresentationSource} from '../../app/src/native-state-presentation.ts';
import {nativePatchBackgroundFrame,nativeTransientPatchFrame} from '../../shared/src/native-state-presentation.ts';
import {PendingPatchController,sourceSnapshot} from './restart24_pending_patch_adapter.mjs';
const folder=resolve('level-editor/work/croisement02-refinement/restart28-approved-climbing-integration-v3');
const sha=b=>createHash('sha256').update(b).digest('hex');
const config=JSON.parse(await readFile(join(folder,'reader-config.json'),'utf8'));
const checked=async p=>{const b=await readFile(p.path);assert.equal(sha(b),p.sha256,p.path);return b;};
for(const p of [config.approval,config.source_manifest,config.recipe,...Object.values(config.models),...Object.values(config.export_reports)])await checked(p);
for(const r of config.native_reader.mapping)await checked({path:r.file,sha256:r.sha256});
let tickCases=0;const results=[];const patches=new Map();
for(const binding of config.bindings){
 const contract=JSON.parse(await checked(binding.contract)),source=JSON.parse(await checked(binding.source_contract));
 validateStateDelivery(contract);assert.deepEqual(contract.native,source.native);assert.equal(contract.families.length,1);
 const family=contract.families[0];assert.deepEqual(family.patch_ids,[binding.id]);assert.equal(family.body_terminal_tick,5);
 for(const state of ['initial','applied']){
  assert.equal(family.physical[state].length,1);const endpoint=family.physical[state][0];assert.equal(endpoint.model,state+'/model.glb');assert.equal(endpoint.model_sha256,config.models[state].sha256);assert.equal(endpoint.model_scene,'climbing-'+state);assert.deepEqual(endpoint.position,[0,0,0]);
 }
 const mission=JSON.parse(await checked(binding.mission_source.data)),level=JSON.parse(await checked(binding.mission_source.level));
 await verifyNativePresentationSource(contract.native,{name:binding.mission_source.name,data:mission,level,camera:binding.mission_source.camera});
 const patch=contract.native.patch_states.find(p=>p.id===binding.id);patches.set(binding.id,patch);
 assert.deepEqual(patch.display_position,[99,0]);assert.equal(patch.final.length,0);assert.equal(patch.integrate_in_background,true);assert.equal(patch.definitive,true);
 for(let tick=0;tick<=8;tick++){
  const snapshot=sourceSnapshot(binding,patch,'forward',tick);tickCases++;
  assert.equal(snapshot.in_transition,tick<5);assert.equal(snapshot.patch_applied,tick>=5);assert.equal(snapshot.background.integrated,tick>=5);
  if(tick<5){assert.equal(snapshot.physical_endpoint_intent.kind,'unmodeled-transition');assert.equal(snapshot.sprite.resource,patch.transition[Math.floor(tick/2)]);}
  else {assert.equal(snapshot.sprite.active,false);assert.equal(snapshot.background.stamp,patch.transition[2]);assert.deepEqual(snapshot.physical_endpoint_intent,family.physical.applied);}
 }
 assert.equal(nativePatchBackgroundFrame(patch,'applied',0),patch.transition[2]);assert.equal(nativeTransientPatchFrame(patch,'applied',0),undefined);
 const controller=new PendingPatchController([binding],new Map([[binding.id,patch]]));controller.selectMission(binding.mission);const initial=controller.snapshot(binding.id);controller.setPhase(binding.id,'forward',5);assert.deepEqual(controller.forceReset(binding.id),initial);assert.throws(()=>controller.setPhase(binding.id,'reverse'));assert.deepEqual(controller.snapshot(binding.id),initial);
 const bad=structuredClone(contract);bad.families[0].body_terminal_tick--;assert.throws(()=>validateStateDelivery(bad),/terminal/);
 const unsafe=structuredClone(contract);unsafe.families[0].physical.initial[0].model='../initial/model.glb';assert.throws(()=>validateStateDelivery(unsafe),/physical source/);
 results.push({id:binding.id,native_contract_exact:true,source_authority_verified:true,terminal_tick:5,transition_frames:3,final_sprite_absent_but_applied_background_retained:true,force_reset_exact:true,definitive_reverse_rejected:true,wrong_terminal_rejected:true,unsafe_model_path_rejected:true});
}
assert.equal(results.length,2);
const report={status:'PASS_PRIVATE_APPROVED_CLIMBING_CPU_CONTRACTS',config_sha256:sha(await readFile(join(folder,'reader-config.json'))),controls:results,tick_cases:tickCases,native_resources_checked:config.native_reader.mapping.length,browser_verified:false,canonical_writes:false,scope:'Two approved physical endpoint bindings only; native transition artwork remains unchanged. CPU guard does not establish exported browser appearance or static substrate exclusion.',runtime_pins:await Promise.all(['level-editor/shared/src/state-delivery.ts','level-editor/shared/src/native-state-presentation.ts','level-editor/app/src/native-state-presentation.ts'].map(async p=>({path:p,sha256:sha(await readFile(p))}))),recipe_sha256:sha(await readFile(new URL(import.meta.url)))};
await writeFile(join(folder,'verification.json'),JSON.stringify(report,null,2)+'\n');console.log(JSON.stringify({status:report.status,controls:results.length,tick_cases:tickCases,native_resources:report.native_resources_checked}));
