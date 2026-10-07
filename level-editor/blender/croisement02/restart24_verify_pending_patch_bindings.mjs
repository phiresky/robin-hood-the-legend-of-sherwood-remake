import assert from 'node:assert/strict';
import { readFile, writeFile } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { createHash } from 'node:crypto';
import { decode } from '../../app/node_modules/fast-png/lib/index.js';
import { validateNativePatchPreview, validateStateDelivery } from '../../shared/src/state-delivery.ts';
import { verifyNativePresentationSource } from '../../app/src/native-state-presentation.ts';
import { PendingPatchController, sourceSnapshot } from './restart24_pending_patch_adapter.mjs';

const root=resolve('.'), folder=resolve('level-editor/work/croisement02-refinement/restart24-hole-mound-bindings-v1');
const sha=bytes=>createHash('sha256').update(bytes).digest('hex');
const json=async path=>JSON.parse(await readFile(path,'utf8'));
const bytes=await readFile(join(folder,'bindings.json')),plan=JSON.parse(bytes);
const verifyPin=async p=>{const bytes=await readFile(join(root,p.path));assert.equal(sha(bytes),p.sha256,p.path);return bytes};
for(const p of [plan.source_catalog,plan.current_map,plan.native_level,plan.recipe,...plan.authorities,...Object.values(plan.candidate_models),...Object.values(plan.missions)])await verifyPin(p);
assert.equal(plan.production_contract,false);assert.equal(plan.publication_allowed,false);
assert.throws(()=>validateStateDelivery(plan));
const level=JSON.parse(await verifyPin(plan.native_level)),patches=new Map(),checks=[],sourceManifest=JSON.parse(await verifyPin(plan.source_manifest));
const resourcePaths=new Map(sourceManifest.resources.map(r=>[r.path,r.source]));
let tickCases=0,initialAbsent=0,appliedAbsent=0;
for(const binding of plan.bindings){
 const contract=JSON.parse(await verifyPin(binding.source_contract));validateNativePatchPreview(contract);
 const mission=JSON.parse(await verifyPin(plan.missions[binding.mission]));
 await verifyNativePresentationSource(contract.native,{name:binding.mission,data:mission,level,camera:{kind:'oblique-orthographic',elevation_deg:35}});
 const patch=contract.native.patch_states.find(p=>p.id===binding.id);patches.set(binding.id,patch);
 assert.equal(patch.source.index,binding.source_patch_index);assert.equal(patch.definitive,binding.definitive);
 const initial=sourceSnapshot(binding,patch,'initial',0),terminal=binding.terminal_tick;
 for(let tick=0;tick<=terminal+3;tick++){
  const s=sourceSnapshot(binding,patch,'forward',tick);tickCases++;
  assert.equal(s.in_transition,tick<terminal);assert.equal(s.patch_applied,tick>=terminal);
  assert.equal(s.background.integrated,patch.integrate_in_background&&tick>=terminal);
  if(binding.aperture)assert.equal(s.aperture.cap,tick>=terminal?'open':'closed');
  if(tick<terminal)assert.equal(s.physical_endpoint_intent.kind,'unmodeled-transition');
  assert.equal(s.physical_render_ready,false);
 }
 const applied=sourceSnapshot(binding,patch,'applied',0);
 assert.equal(applied.sprite.active,patch.final.length>0);
 if(binding.endpoints.initial.kind==='source-absent'){
  initialAbsent++;const frame=patch.initial[0],raw=await readFile(resourcePaths.get(frame.path)??join(root,'level-editor/library',frame.path));assert.equal(sha(raw),frame.sha256);
  const png=decode(raw);assert.equal(png.channels,4);assert(!png.data.some((v,i)=>i%4===3&&v!==0),'Absent initial has painted alpha');
 }
 if(binding.endpoints.applied.kind==='source-absent'){
  appliedAbsent++;assert.equal(binding.id,'mission-Tac19_FoB_EC-patch-015');assert.equal(applied.sprite.active,false);assert.equal(applied.background.integrated,false);
 }
 const controller=new PendingPatchController([binding],new Map([[binding.id,patch]]));controller.selectMission(binding.mission);
 controller.setPhase(binding.id,'forward',terminal);assert.deepEqual(controller.forceReset(binding.id),initial);
 const before=controller.snapshot(binding.id);
 if(binding.definitive){assert.throws(()=>controller.setPhase(binding.id,'reverse'));assert.deepEqual(controller.snapshot(binding.id),before)}
 assert.throws(()=>controller.setPhase(binding.id,'forward',-1));assert.deepEqual(controller.snapshot(binding.id),before);
 const dispatch=[];controller.dispatchSource({mission:binding.mission,setPatchState:(...args)=>dispatch.push(args)},binding.id);assert.deepEqual(dispatch,[[binding.id,'initial',0]]);assert.throws(()=>controller.dispatchSource({mission:'other',setPatchState(){}},binding.id));
 assert.throws(()=>controller.physicalBinding());
 const changed=structuredClone(binding);changed.terminal_tick++;assert.throws(()=>sourceSnapshot(changed,patch,'initial',0));
 checks.push({id:binding.id,initial_sha256:sha(JSON.stringify(initial)),applied_sha256:sha(JSON.stringify(applied)),terminal_tick:terminal,
   reset_exact:true,definitive_reverse_atomic:binding.definitive,unknown_physical_motion_preserved:true,ready:false});
}
assert.equal(checks.length,63);assert.equal(initialAbsent,1);assert.equal(appliedAbsent,1);
const controller=new PendingPatchController(plan.bindings,patches);
for(const mission of Object.keys(plan.missions)){
 controller.selectMission(mission);const rows=plan.bindings.filter(b=>b.mission===mission);
 const initial=rows.map(b=>controller.snapshot(b.id));
 for(const b of rows)controller.setPhase(b.id,'forward',b.terminal_tick);
 const forward=rows.map(b=>controller.snapshot(b.id));controller.resetMission();
 for(const b of [...rows].reverse())controller.setPhase(b.id,'forward',b.terminal_tick);
 assert.deepEqual(rows.map(b=>controller.snapshot(b.id)),forward);
 for(const b of rows)controller.forceReset(b.id);
 assert.deepEqual(rows.map(b=>controller.snapshot(b.id)),initial);
 const other=plan.bindings.find(b=>b.mission!==mission);assert.throws(()=>controller.setPhase(other.id,'applied'));
}
controller.clear();assert.throws(()=>controller.snapshot(plan.bindings[0].id));
for(const p of [plan.source_catalog,plan.current_map,...Object.values(plan.candidate_models)])await verifyPin(p);
const result={status:'PASS_PRIVATE_SOURCE_TRANSITION_PLUMBING_NOT_PHYSICAL_DELIVERY',bindings_sha256:sha(bytes),bindings:63,tick_cases:tickCases,
 exact_absent_initial:initialAbsent,exact_absent_applied:appliedAbsent,mission_order_independent:true,reset_exact:true,geometry_approval_unresolved:true,
 runtime_pins:await Promise.all(['level-editor/shared/src/state-delivery.ts','level-editor/shared/src/native-state-presentation.ts','level-editor/app/src/native-state-presentation.ts'].map(async path=>({path,sha256:sha(await readFile(path))}))),
 recipe_pins:await Promise.all(['level-editor/blender/croisement02/restart24_pending_patch_adapter.mjs','level-editor/blender/croisement02/restart24_verify_pending_patch_bindings.mjs'].map(async path=>({path,sha256:sha(await readFile(path))}))),checks};
await writeFile(join(folder,'verification.json'),JSON.stringify(result,null,2)+'\n');console.log(JSON.stringify({status:result.status,bindings:63,tick_cases:tickCases,absent:[initialAbsent,appliedAbsent]}));
