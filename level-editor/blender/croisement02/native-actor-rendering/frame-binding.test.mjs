import test from 'node:test';
import assert from 'node:assert/strict';
import * as THREE from '../../../app/node_modules/three/build/three.module.js';
import { createActorFrameBinding } from './frame-binding.mjs';

const identity = 'mission-hash:soldiers:0';
function frame(left = -17, top = 40, width = 36, height = 40, shadow = true) {
  return { geometry: new THREE.PlaneGeometry(width, height), texture: new THREE.Texture(),
    shadow: shadow ? new THREE.Texture() : null, bounds: {left, top, width, height} };
}
function setup() {
  const initial = frame();
  const body = new THREE.Mesh(initial.geometry, new THREE.MeshBasicMaterial({map: initial.texture}));
  const binding = createActorFrameBinding(THREE, body, identity);
  const snapshot = { identity, frame: initial, anchor: [100,0,200], rotation: 0, active: true,
    elevation: Math.PI / 5, supportHeight: () => 0, shadowStyle: {color:0, opacity:0.4} };
  binding.apply(snapshot);
  return {body, binding, snapshot};
}
test('directional frame replacement updates common bounds, body and shadow together', () => {
  const {body,binding,snapshot} = setup();
  const old = binding.shadow;
  let disposed = 0, borrowedDisposed = 0;
  old.geometry.addEventListener('dispose',()=>disposed++);
  old.material.addEventListener('dispose',()=>disposed++);
  snapshot.frame.shadow.addEventListener('dispose',()=>borrowedDisposed++);
  snapshot.frame.geometry.addEventListener('dispose',()=>borrowedDisposed++);
  const next = frame(-25,33,52,48);
  binding.apply({...snapshot,frame:next,rotation:1.2});
  assert.equal(body.geometry,next.geometry); assert.equal(body.material.map,next.texture);
  assert.equal(binding.shadow.material.map,next.shadow);
  assert.equal(binding.shadow.rotation.y,-1.2);
  assert.equal(binding.shadow.geometry.getAttribute('position').getX(0),-25);
  assert.equal(old.parent,null); assert.equal(disposed,2); assert.equal(borrowedDisposed,0);
  assert.equal(body.children.length,1);
  binding.dispose(); binding.dispose();
  assert.equal(body.children.length,0); assert.equal(borrowedDisposed,0);
});
test('missing shadow, inactive identity and hidden body cannot retain stale shadow', () => {
  const {body,binding,snapshot} = setup();
  binding.apply({...snapshot,frame:frame(-17,40,36,40,false)});
  assert.equal(binding.shadow,null); assert.equal(body.visible,true);
  binding.apply(snapshot); binding.apply({...snapshot,active:false});
  assert.equal(binding.shadow,null); assert.equal(body.visible,false);
  binding.apply({...snapshot,frame:null}); assert.equal(body.visible,false);
  binding.apply(snapshot); assert.equal(body.visible,true); assert.ok(binding.shadow);
  binding.dispose();
});
test('support failure leaves the prior body, map, placement and shadow intact', () => {
  const {body,binding,snapshot} = setup();
  const shadow = binding.shadow, geometry = body.geometry, map = body.material.map;
  let calls = 0;
  assert.throws(()=>binding.apply({...snapshot,frame:frame(-2,5,8,9),anchor:[1,2,3],
    supportHeight:()=>{ if (++calls===3) throw Error('missing support'); return 5; }}),/missing support/);
  assert.equal(body.geometry,geometry); assert.equal(body.material.map,map);
  assert.deepEqual(body.position.toArray(),snapshot.anchor);
  assert.equal(binding.shadow,shadow); assert.equal(body.children.length,1);
  assert.throws(()=>binding.apply({...snapshot,identity:'different'}),/identity/);
  binding.dispose(); assert.throws(()=>binding.apply(snapshot),/disposed/);
});
test('same frame at a new anchor recomputes support and keeps world shadow orientation', () => {
  const {body,binding,snapshot} = setup();
  const calls = [];
  const supportHeight = (x,y)=>{calls.push([x,y]);return x/10+y/20;};
  binding.apply({...snapshot,anchor:[300,10,400],rotation:0.75,supportHeight});
  const positions = binding.shadow.geometry.getAttribute('position');
  assert.equal(calls.length,4); assert.equal(calls[0][0],283);
  const base = 10*Math.cos(snapshot.elevation);
  const expectedY = (supportHeight(...calls[0])-base)/Math.cos(snapshot.elevation)+0.15;
  assert.ok(Math.abs(positions.getY(0)-expectedY)<1e-5);
  body.updateMatrixWorld(true);
  const orientation = new THREE.Quaternion(); binding.shadow.getWorldQuaternion(orientation);
  assert.ok(orientation.angleTo(new THREE.Quaternion())<1e-7);
  binding.dispose();
});
test('failed preparation disposes scratch geometry without disposing borrowed textures', () => {
  let allocated = 0, released = 0, borrowedReleased = 0;
  class TrackedGeometry extends THREE.PlaneGeometry {
    constructor(...args) { super(...args); allocated++; this.addEventListener('dispose',()=>released++); }
  }
  const next = frame();
  next.texture.addEventListener('dispose',()=>borrowedReleased++);
  next.shadow.addEventListener('dispose',()=>borrowedReleased++);
  const body = new THREE.Mesh(next.geometry,new THREE.MeshBasicMaterial({map:next.texture}));
  const binding = createActorFrameBinding({...THREE,PlaneGeometry:TrackedGeometry},body,identity);
  assert.throws(()=>binding.apply({identity,frame:next,anchor:[0,0,0],rotation:0,active:true,
    elevation:Math.PI/5,supportHeight:()=>NaN,shadowStyle:{color:0,opacity:0.4}}),/support height/);
  assert.equal(allocated,1); assert.equal(released,1); assert.equal(borrowedReleased,0);
  assert.equal(body.children.length,0); assert.equal(binding.shadow,null);
  binding.dispose(); assert.equal(released,1);
});
