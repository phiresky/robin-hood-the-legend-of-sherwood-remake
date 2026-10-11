/** Check production state timing with real artwork resources and explicit CPU-only model roots. */
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import * as THREE from '../../app/node_modules/three/build/three.module.js';
import {StateDelivery} from '../../app/src/state-delivery.ts';
const root=path.resolve(import.meta.dirname,'../../..'),base=path.join(root,'level-editor/work/york-refinement/restart2/restart48-production-gate-delivery-v2');
const read=p=>JSON.parse(fs.readFileSync(p));
const request=read(path.join(base,'production-browser-request.json')),contract=read(request.contract.path),source={name:request.source.name,data:read(request.source.mission.path),level:read(request.source.level.path),camera:{kind:'oblique-orthographic',elevation_deg:35}};
const sha=p=>createHash('sha256').update(fs.readFileSync(p)).digest('hex');
for(const pin of [request.contract,...request.pins,...request.runtime,request.source.mission,request.source.level])assert.equal(sha(pin.path),pin.sha256);
const roots=[];
const delivery=new StateDelivery(()=>({load:async()=>{const group=new THREE.Group();roots.push(group);return group;},dispose(){}}));
await delivery.set(contract,source,{},resource=>Promise.resolve(new Uint8Array(fs.readFileSync(path.join(request.library,resource.path)))));
delivery.selectMode('physical-motion');
const [covered,raised]=delivery.physical.children,family=contract.families[0],checks=[];
assert.equal(delivery.familyTick(family.id),undefined);delivery.activate(family.id);delivery.setPlaying(true);
for(let tick=0;tick<92;tick++){
 const frame=Math.min(44,Math.floor(tick/2)),applied=tick>=89;
 assert.equal(delivery.familyTick(family.id),tick);assert.equal(covered.visible,!applied);assert.equal(raised.visible,applied);
 assert.deepEqual(covered.position.toArray(),applied?[0,0,0]:family.physical.motion.translation_frames[frame]);assert.deepEqual(raised.position.toArray(),[0,0,0]);
 for(const state of [covered,raised])assert.deepEqual(state.children[0].position.toArray(),family.physical.initial[0].position);
 const artwork=delivery.native.pixels();assert.equal(artwork.width,220);assert.equal(artwork.height,250);
 checks.push({tick,frame,endpoint:applied?'applied':'initial',translation:covered.position.toArray()});delivery.advance(1/25);
}
delivery.reset(family.id);assert.equal(covered.visible,true);assert.deepEqual(covered.position.toArray(),[0,0,0]);delivery.dispose();assert.equal(delivery.physical.children.length,0);
const report={status:'PASS_PRODUCTION_TIMING_WITH_CPU_MODEL_ROOTS',contract:request.contract,runtime:request.runtime,checks,source_resources_verified:true,reset_and_dispose:true,scope:'Actual StateDelivery and artwork resource decoding; CPU empty roots stand in for models. Not a model appearance, viewport rendering, engine snapshot transport or gameplay proof.'};
fs.writeFileSync(path.join(base,'production-cpu-report.json'),JSON.stringify(report,null,2)+'\n');console.log(JSON.stringify({status:report.status,ticks:checks.length}));
