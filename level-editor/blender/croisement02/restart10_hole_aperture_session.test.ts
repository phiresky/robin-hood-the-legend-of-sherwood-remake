import test from 'node:test';
import assert from 'node:assert/strict';
import * as THREE from '../../app/node_modules/three/build/three.module.js';
import {PrivateApertureSession} from './restart10_hole_aperture_session.ts';
import {isEffectivelyVisible} from '../../app/src/patch-display.ts';
function fixture(){
 const root=new THREE.Group(),original=new THREE.Group(),hiddenOriginal=new THREE.Group();hiddenOriginal.visible=false;
 const caps=['a','b'].map(id=>{const o=new THREE.Group();o.userData.reveal_hide_when_applied=id==='a'?['a','c']:[id];root.add(o);return o;});
 const endpoints=['a','b'].map(id=>{const parent=new THREE.Group(),initial=new THREE.Group(),applied=new THREE.Group();initial.userData.reveal_hide_when_applied=id==='a'?['a','c']:[id];applied.userData.reveal_show_when_applied=id==='a'?['a','c']:[id];parent.add(initial,applied);root.add(parent);return{parent,initial,applied};});
 const session=new PrivateApertureSession(root,[original,hiddenOriginal],[{patch:'a',mission:'one',endpointParent:endpoints[0]!.parent},{patch:'b',mission:'one',endpointParent:endpoints[1]!.parent},{patch:'c',mission:'two',endpointParent:endpoints[0]!.parent}]);
 return{root,original,hiddenOriginal,caps,endpoints,session};
}
test('two simultaneous holes reset independently and restore original receiver on leaving physical mode',()=>{
 const f=fixture();assert.throws(()=>f.session.selectPhysical(true),/Select a mission/);
 f.session.selectMission('one');f.session.selectPhysical(true);assert(!f.original.visible);assert(!f.hiddenOriginal.visible);
 f.session.setApplied('a',true);f.session.setApplied('b',true);assert(f.caps.every(c=>!c.visible));assert(f.endpoints.every(e=>isEffectivelyVisible(e.applied)));
 f.session.setApplied('a',false);assert(f.caps[0]!.visible);assert(!f.caps[1]!.visible);assert(isEffectivelyVisible(f.endpoints[1]!.applied));
 f.session.reset();assert(f.caps.every(c=>c.visible));assert(f.endpoints.every(e=>isEffectivelyVisible(e.initial)));
 f.session.selectPhysical(false);assert(f.original.visible);assert(!f.hiddenOriginal.visible);assert(!f.root.visible);
 f.session.dispose();assert(f.original.visible);assert(!f.hiddenOriginal.visible);assert.throws(()=>f.session.reset(),/disposed/);
});
test('mission change clears aperture states and prevents inactive mission effects',()=>{
 const f=fixture();f.session.selectMission('one');f.session.selectPhysical(true);f.session.setApplied('a',true);
 assert.throws(()=>f.session.setApplied('c',true),/outside/);f.session.selectMission('two');assert(f.caps.every(c=>c.visible));assert(!isEffectivelyVisible(f.endpoints[1]!.initial));assert.throws(()=>f.session.setApplied('a',true),/outside/);f.session.setApplied('c',true);assert(!f.caps[0]!.visible);assert(isEffectivelyVisible(f.endpoints[0]!.applied));
 f.session.dispose();assert(f.original.visible);
});
test('initial user-hidden receiver is restored exactly and disposal is idempotent',()=>{
 const f=fixture();f.session.selectMission('one');f.session.selectPhysical(true);f.session.setApplied('b',true);f.session.dispose();f.session.dispose();assert.deepEqual([f.original.visible,f.hiddenOriginal.visible],[true,false]);
});
