import assert from 'node:assert/strict';
import {readFile,writeFile} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import {createHash} from 'node:crypto';
import * as THREE from '../../app/node_modules/three/build/three.module.js';
import {StateAperturePreview} from '../../app/src/state-aperture-preview.ts';
import {isEffectivelyVisible} from '../../app/src/patch-display.ts';
import {validateStateDelivery} from '../../shared/src/state-delivery.ts';
import {prepareCurrentHoleApertures} from './restart39_hole_aperture_stage.ts';
const folder=resolve(process.argv[2] ?? 'level-editor/work/active/croisement02/restart39-current-hole-apertures-v1');
const sha=b=>createHash('sha256').update(b).digest('hex');
const checked=async pin=>{const bytes=await readFile(pin.path);assert.equal(sha(bytes),pin.sha256,pin.path);return bytes;};
const integration=JSON.parse(await readFile(join(folder,process.argv[3] ?? 'integration-focused-v2.json'),'utf8'));
for(const pin of [integration.map,integration.receiver_partition,integration.endpoint_config,integration.endpoint_approval,integration.recipe])await checked(pin);
// Lifecycle checks need the exact GLB node hierarchy, transforms and reveal extras; no image/render claim.
async function hierarchy(pin){
 const bytes=await checked(pin),length=bytes.readUInt32LE(12),doc=JSON.parse(bytes.subarray(20,20+length).toString());
 const build=i=>{const n=doc.nodes[i];const o=new THREE.Group();o.name=n.name??'';o.userData=structuredClone(n.extras??{});if(n.matrix){o.matrix.fromArray(n.matrix);o.matrix.decompose(o.position,o.quaternion,o.scale)}else{if(n.translation)o.position.fromArray(n.translation);if(n.rotation)o.quaternion.fromArray(n.rotation);if(n.scale)o.scale.fromArray(n.scale)}for(const c of n.children??[])o.add(build(c));return o;};
 return{sha256:pin.sha256,scenes:doc.scenes.map(s=>{const o=new THREE.Group();o.name=s.name??'';for(const n of s.nodes)o.add(build(n));return o})};
}
const stage=new THREE.Group(),receivers=new Map();
for(const [i,r]of integration.receivers.entries()){
 const model=await hierarchy(r.model),original=model.scenes.find(s=>s.name===r.original_scene).clone(true);original.rotation.x=Math.PI/2;original.visible=i===0;stage.add(original);receivers.set(r.asset,{original,model});
}
const endpoints={};for(const state of ['initial','applied'])endpoints[state]=await hierarchy(integration.endpoints[state].model);
const visibility=[...receivers.values()].map(r=>r.original.visible);stage.updateMatrixWorld(true);const matrices=[...receivers.values()].map(r=>r.original.matrixWorld.toArray());const results=[];
for(const control of integration.controls){
 const contract=JSON.parse(await checked(control.contract)),source=JSON.parse(await checked(control.source_contract));validateStateDelivery(contract);assert.deepEqual(contract.native,source.native);assert.equal(contract.families.length,1);assert.equal(contract.families[0].id,control.family);assert.equal(contract.families[0].body_terminal_tick,control.terminal_tick);
 const prepared=prepareCurrentHoleApertures(integration,receivers,endpoints,control.id);stage.add(prepared.root);stage.updateMatrixWorld(true);assert.equal(prepared.bindings.length,1);
 const caps=site=>{const values=[];prepared.root.traverse(o=>{if(o.userData.aperture_component===site)values.push(o)});assert.ok(values.length);return values;};const parent=site=>prepared.root.getObjectByName(site);
 const preview=new StateAperturePreview(prepared);preview.selectMission(control.mission);preview.selectEnabled(true);assert.ok(prepared.originalReceivers.every(o=>!o.visible));
 for(const s of integration.sites){assert.ok(caps(s.id).every(isEffectivelyVisible));assert.equal(parent(s.id).visible,integration.controls.some(c=>c.mission===control.mission&&c.site===s.id));}
 preview.selectEndpoint(control.family,'applied');assert.ok(caps(control.site).every(o=>!isEffectivelyVisible(o)));assert.ok(isEffectivelyVisible(parent(control.site).children[1]));assert.ok(!isEffectivelyVisible(parent(control.site).children[0]));
 for(const s of integration.sites.filter(s=>s.id!==control.site))assert.ok(caps(s.id).every(isEffectivelyVisible));
 preview.selectEndpoint(control.family,'initial');assert.ok(caps(control.site).every(isEffectivelyVisible));assert.ok(isEffectivelyVisible(parent(control.site).children[0]));assert.ok(!isEffectivelyVisible(parent(control.site).children[1]));
 const other=integration.controls.find(c=>c.id!==control.id);assert.throws(()=>preview.selectEndpoint(other.family,'applied'),/Unknown/);
 preview.selectEnabled(false);assert.deepEqual(prepared.originalReceivers.map(o=>o.visible),visibility);assert.ok(!isEffectivelyVisible(prepared.root));preview.dispose();assert.deepEqual(prepared.originalReceivers.map(o=>o.visible),visibility);assert.equal(prepared.root.parent,null);assert.deepEqual(prepared.originalReceivers.map(o=>o.matrixWorld.toArray()),matrices);
 results.push({id:control.id,mission:control.mission,site:control.site,cap_apply_reset_exact:true,unrelated_caps_unchanged:true,other_controls_rejected:true,native_contract_exact:true,native_visibility_and_retirement_exact:true});
}
assert.equal(results.length,30);
const report={status:'PASS_CURRENT_RECEIVER_FOCUSED_CAP_LIFECYCLE',integration_sha256:sha(await readFile(join(folder,process.argv[3] ?? 'integration-focused-v2.json'))),sites:17,controls:results,missions:7,independent_cap_reset:true,focus_only_no_background_domain_union:true,native_visibility_restore:true,dispose_and_original_transforms_exact:true,current_map_pin_verified:true,browser_verified:false,appearance_scope:'No render claim; actual GLB node hierarchy/transforms/extras exercised through production StateAperturePreview.',canonical_writes:false,recipe_sha256:sha(await readFile(new URL(import.meta.url)))};await writeFile(join(folder,'lifecycle-focused-verification-v2.json'),JSON.stringify(report,null,2)+'\n');console.log(JSON.stringify({status:report.status,controls:results.length,sites:17,missions:7}));
