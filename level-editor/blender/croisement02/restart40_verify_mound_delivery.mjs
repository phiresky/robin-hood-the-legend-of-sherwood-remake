import assert from 'node:assert/strict';
import {readFile,writeFile} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import {createHash} from 'node:crypto';
import {validateStateDelivery} from '../../shared/src/state-delivery.ts';
const folder=resolve(process.argv[2] ?? 'level-editor/work/active/croisement02/restart40-approved-mound-export-v2');
const sha=b=>createHash('sha256').update(b).digest('hex');
const checked=async p=>{const bytes=await readFile(p.path);assert.equal(sha(bytes),p.sha256,p.path);return bytes;};
const config=JSON.parse(await readFile(join(folder,'reader-config.json'),'utf8'));
for(const p of [config.approval,config.parent_bindings,config.recipe,...Object.values(config.exports)])await checked(p);
for(const r of config.native_reader.mapping)await checked({path:r.file,sha256:r.sha256});
const before=JSON.parse(await checked(config.parent_bindings));const results=[];const scenes=new Map();
for(const row of config.bindings){
 const contract=JSON.parse(await checked(row.contract)),source=JSON.parse(await checked(row.source_contract));validateStateDelivery(contract);assert.deepEqual(contract.native,source.native);assert.equal(contract.families.length,1);const family=contract.families[0];assert.deepEqual(family.patch_ids,[row.id]);assert.equal(family.body_terminal_tick,row.terminal_tick);
 const prior=before.bindings.find(b=>b.id===row.id);assert.equal(prior.endpoints.initial.selection.site,row.site);assert.equal(prior.transition_duration,row.transition_duration);
 for(const state of ['initial','applied']){
  const endpoints=family.physical[state];
  if(state==='applied'&&prior.endpoints.applied.kind==='source-absent'){
   assert.equal(row.id,'mission-Tac19_FoB_EC-patch-015');assert.deepEqual(endpoints,{kind:'absent'});const missing=structuredClone(contract);missing.families[0].physical.applied=[];assert.throws(()=>validateStateDelivery(missing),/missing physical endpoint/);const patch=contract.native.patch_states.find(p=>p.id===row.id);assert.deepEqual(patch.final,[]);assert.equal(patch.integrate_in_background,false);continue;
  }
  assert.equal(endpoints.length,1);const e=endpoints[0];assert.deepEqual(e.position,[0,0,0]);
  if(!scenes.has(e.model)){
   const raw=await checked({path:config.asset_handle.mapping[e.model],sha256:e.model_sha256});const doc=JSON.parse(raw.subarray(20,20+raw.readUInt32LE(12)).toString());scenes.set(e.model,doc.scenes.map(s=>s.name));
  }
  assert.ok(scenes.get(e.model).includes(e.model_scene));
  if(state==='initial')assert.equal(e.model_scene,row.site+'-initial');
  else assert.equal(e.model_scene,prior.endpoints.applied.private_runtime_candidate.model_scene);
 }
 const bad=structuredClone(contract);bad.families[0].body_terminal_tick--;assert.throws(()=>validateStateDelivery(bad),/terminal/);
 const unsafe=structuredClone(contract);unsafe.families[0].physical.initial[0].model='../model.glb';assert.throws(()=>validateStateDelivery(unsafe),/physical source/);
 results.push({id:row.id,site:row.site,native_contract_exact:true,world_placement_exact:true,approved_export_scene_present:true,wrong_terminal_rejected:true,unsafe_model_path_rejected:true});
}
assert.equal(results.length,32);assert.equal(new Set(results.map(r=>r.site)).size,20);assert.ok(!results.some(r=>r.id==='mission-Tac19_FoB_EC-patch-000'));
const report={status:'PASS_PRIVATE_APPROVED_MOUND_CONTRACTS',config_sha256:sha(await readFile(join(folder,'reader-config.json'))),controls:results,models:scenes.size,native_resources:config.native_reader.mapping.length,orphan_absent_initial_untouched:true,browser_verified:false,canonical_writes:false,recipe_sha256:sha(await readFile(new URL(import.meta.url)))};
await writeFile(join(folder,'verification.json'),JSON.stringify(report,null,2)+'\n');console.log(JSON.stringify({status:report.status,controls:results.length,sites:20}));
