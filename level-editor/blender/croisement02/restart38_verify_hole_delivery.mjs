import assert from 'node:assert/strict';
import {readFile,writeFile} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import {createHash} from 'node:crypto';
import {validateStateDelivery} from '../../shared/src/state-delivery.ts';
import {PendingPatchController,sourceSnapshot} from './restart24_pending_patch_adapter.mjs';
const folder=resolve('level-editor/work/croisement02-refinement/restart38-approved-hole-export-v1');
const sha=b=>createHash('sha256').update(b).digest('hex');
const config=JSON.parse(await readFile(join(folder,'reader-config.json'),'utf8'));
const checked=async p=>{const b=await readFile(p.path);assert.equal(sha(b),p.sha256,p.path);return b;};
for(const p of [config.approval,config.parent_bindings,config.recipe,...Object.values(config.models)])await checked(p);
for(const r of config.native_reader.mapping)await checked({path:r.file,sha256:r.sha256});
const before=JSON.parse(await checked(config.parent_bindings)),after=JSON.parse(await readFile(join(folder,'bindings-private.json'),'utf8'));assert.equal(before.bindings.length,63);assert.equal(after.bindings.length,63);
let tickCases=0;const results=[];
for(const row of config.bindings){
 const contract=JSON.parse(await checked(row.contract)),source=JSON.parse(await checked(row.source_contract));validateStateDelivery(contract);assert.deepEqual(contract.native,source.native);assert.equal(contract.families.length,1);const family=contract.families[0];assert.deepEqual(family.patch_ids,[row.id]);assert.equal(family.body_terminal_tick,row.terminal_tick);
 const old=before.bindings.find(b=>b.id===row.id),next=after.bindings.find(b=>b.id===row.id);assert.ok(old&&next);const patch=contract.native.patch_states.find(p=>p.id===row.id);
 for(const state of ['initial','applied']){const endpoint=family.physical[state][0];assert.equal(family.physical[state].length,1);assert.equal(endpoint.model,state+'/model.glb');assert.equal(endpoint.model_sha256,config.models[state].sha256);assert.equal(endpoint.model_scene,'endpoint-'+state);const[x,y,z]=old.endpoints[state].placement.value;assert.deepEqual(endpoint.position,[x,z,-y]);assert.deepEqual(next.endpoints[state].runtime_asset,endpoint);}
 for(const phase of ['initial','forward','applied',...(patch.definitive?[]:['reverse'])])for(let tick=0;tick<=row.transition_duration+2;tick++){
  const a=sourceSnapshot(old,patch,phase,tick),b=sourceSnapshot(next,patch,phase,tick);delete a.physical_endpoint_intent;delete b.physical_endpoint_intent;assert.deepEqual(a,b);assert.equal(b.physical_render_ready,false);tickCases++;
 }
 const controller=new PendingPatchController([next],new Map([[row.id,patch]]));controller.selectMission(row.mission);const initial=controller.snapshot(row.id);controller.setPhase(row.id,'forward',row.terminal_tick);assert.deepEqual(controller.forceReset(row.id),initial);
 const bad=structuredClone(contract);bad.families[0].body_terminal_tick--;assert.throws(()=>validateStateDelivery(bad),/terminal/);
 const unsafe=structuredClone(contract);unsafe.families[0].physical.initial[0].model='../initial/model.glb';assert.throws(()=>validateStateDelivery(unsafe),/physical source/);
 results.push({id:row.id,native_contract_exact:true,timing_and_aperture_intent_exact:true,placement_axis_conversion_exact:true,force_reset_exact:true,wrong_terminal_rejected:true,unsafe_model_path_rejected:true});
}
assert.equal(results.length,30);const ids=new Set(results.map(r=>r.id));for(const binding of before.bindings)if(!ids.has(binding.id))assert.deepEqual(binding,after.bindings.find(b=>b.id===binding.id));
const report={status:'PASS_PRIVATE_APPROVED_HOLE_CPU_CONTRACTS',config_sha256:sha(await readFile(join(folder,'reader-config.json'))),controls:results,tick_cases:tickCases,native_resources_checked:config.native_reader.mapping.length,other_33_bindings_unchanged:true,browser_verified:false,current_apertures_verified:false,canonical_writes:false,scope:'Approved hole endpoint appearance contracts only. Current receiver aperture integration and production WebGL appearance remain separate.',recipe_sha256:sha(await readFile(new URL(import.meta.url)))};await writeFile(join(folder,'verification.json'),JSON.stringify(report,null,2)+'\n');console.log(JSON.stringify({status:report.status,controls:results.length,tick_cases:tickCases,native_resources:report.native_resources_checked}));
