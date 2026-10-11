/** Actual loader/controller proof for one independently triggered reviewed mechanism. */
import * as THREE from '../../app/node_modules/three/build/three.module.js';
import {StateDelivery} from '../../app/src/state-delivery.ts';
import {NativeStatePresentation} from '../../app/src/native-state-presentation.ts';
import {loadProductionGateInputs} from './restart56_production_gate_browser_fixture.mjs';
const check=(condition,label)=>{if(!condition)throw new Error(label);};
const equal=(a,b,label)=>check(JSON.stringify(a)===JSON.stringify(b),label);

export async function runProductionWinchBrowserFixture({request,resolveURL,views,capture}) {
 check(views?.length===8&&typeof capture==='function','Eight saved cameras and renderer capture required');
 const input=await loadProductionGateInputs(request,resolveURL),delivery=new StateDelivery(),control=new NativeStatePresentation();
 const family=input.contract.families[0];
 check(input.contract.families.length===1&&family.id==='york-winch','Wrong independent mechanism family');
 const motion=family.physical.motion;
 check(motion.kind==='native-patch-node-poses'&&motion.node_frames.length===45&&family.body_terminal_tick===89,'Wrong reviewed motion');
 const captures=[],checks=[],seen=new Set();
 try {
  check(await delivery.set(input.contract,input.source,input.library,input.read),'Actual delivery adoption failed');
  check(await control.set(input.contract.native,input.source,input.read),'Native control adoption failed');
  control.setPlaying(true);delivery.selectMode('physical-motion');
  const [initial,applied]=delivery.physical.children;
  check(initial.children.length===0&&applied.children.length===1,'Transparent initial must not load a placeholder');
  const pieces=motion.node_frames[0].map(({node})=>{const found=[];applied.traverse(o=>{if(o.name===node)found.push(o)});check(found.length===1&&found[0].isMesh,'Actual mesh missing '+node);return found[0]});
  check(pieces.length===109,'Incomplete mechanism');
  const bases=pieces.map(o=>o.matrix.clone()),positions=pieces.map(o=>o.position.toArray()),roots={physical:delivery.physical,initial,applied};
  const matrix=new THREE.Matrix4();
  const verify=tick=>{
   const absent=tick===undefined,frame=absent?44:Math.min(44,Math.floor(tick/2));
   check(applied.visible===!absent,'Mechanism visibility differs from native phase');
   check(initial.visible===absent,'Initial empty wrapper state differs');
   equal(applied.position.toArray(),[0,0,0],'Model placement applied twice');
   pieces.forEach((object,index)=>{
    const expected=bases[index].clone();
    if(!absent&&tick<89)expected.premultiply(matrix.fromArray(motion.node_frames[frame][index].matrix));
    check(object.matrix.elements.every((v,i)=>Math.abs(v-expected.elements[i])<1e-9),'Pose mismatch '+object.name+' at '+tick);
    equal(object.position.toArray(),positions[index],'Original node TRS was mutated');
   });
   if(!absent)seen.add(frame);
   checks.push({tick:tick??null,frame,visible:!absent,nodes:pieces.length});
  };
  const save=async(label,tick)=>{for(const index of [0,3]){await capture({label:label+'-view'+index,view:views[index],phase:label,tick,roots,delivery});captures.push(label+'-view'+index)}};
  verify(undefined);await save('initial-absent',undefined);
  delivery.activate('york-winch');delivery.setPlaying(true);
  for(let tick=0;tick<92;tick++){
   if(tick){delivery.advance(1/25);control.advance(1/25)}
   check(delivery.familyTick('york-winch')===tick,'Native clock drift');verify(tick);
   if([0,44,70,72,76,88,89].includes(tick))await save('tick'+tick,tick);
  }
  check(seen.size===45,'Not all native poses exercised');
  delivery.reset('york-winch');verify(undefined);await save('reset-absent',undefined);
  equal([...delivery.native.compose().data],[...control.compose().data],'Native reset differs from independent initial at same ambient time');
  delivery.selectMode('native-art');check(!delivery.physical.visible,'Native selection kept physical mechanism visible');
  delivery.dispose();check(!delivery.physical.children.length&&!delivery.ready,'Dispose retained physical state');
  return {status:'EXECUTED_ACTUAL_LOADER_MOTION_PENDING_VISUAL_REVIEW',checks,captures,poses:seen.size,updates:92,nodes:pieces.length,verified_resources:input.verifiedResources(),scope:'Isolated complete mechanism. Room patch005, gate patch000, normal editor catalog and gameplay engine integration remain separate.'};
 } finally {delivery.dispose();control.dispose()}
}
