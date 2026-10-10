import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import {createRequire} from 'node:module';
import {createHash} from 'node:crypto';
import {createPortcullisPoseAdapter} from './restart45_portcullis_pose_adapter.mjs';
const require = createRequire(new URL('../../app/package.json', import.meta.url));
const THREE = require('three');
const bytes = fs.readFileSync(new URL('../../work/york-refinement/restart2/jamb-clearance-candidate-v1/motion-proposal.json', import.meta.url));
assert.equal(createHash('sha256').update(bytes).digest('hex'), '07fd102ae0a508bf77ff10f6d52dda6c746723d3da7011b312b9164b7758a446');
const motion = JSON.parse(bytes);
const origin = [2357.217041015625, 109.8709487915039, 1801.3960571289062];
const initial = {patch:'patch-000', applied:false, inTransition:false, fxActive:true};
function fixture() {
  const covered = new THREE.Group(), raised = new THREE.Group();
  covered.rotation.x = -.5 * Math.PI; raised.rotation.x = -.5 * Math.PI;
  covered.position.set(1,2,3); raised.position.set(1,2,3);
  covered.scale.set(1,2,3); raised.scale.set(1,2,3);
  return {covered, raised, adapter:createPortcullisPoseAdapter(covered, raised, motion, origin)};
}
test('applied endpoint persists with FX stopped and is not lifted twice', () => {
  const f=fixture();
  f.adapter.apply({...initial, applied:true, fxActive:false});
  assert.equal(f.covered.visible,false); assert.equal(f.raised.visible,true);
  assert.deepEqual(f.raised.position.toArray(), origin.map((x,i)=>x+i+1));
});
test('all approved poses retain basis and scale; repeated updates do not accumulate translation', () => {
  const f=fixture(), q=f.covered.quaternion.toArray(), scale=f.covered.scale.toArray();
  for (let frame=0;frame<45;frame++) {
    const snapshot={...initial,inTransition:true,frame,counter:0};
    f.adapter.apply(snapshot); const position=f.covered.position.toArray(); f.adapter.apply(snapshot);
    assert.deepEqual(f.covered.position.toArray(),position);
    assert.equal(position[1],2+origin[1]+motion.rows[frame].nominal_lift_world_z);
    assert.deepEqual(f.covered.quaternion.toArray(),q); assert.deepEqual(f.covered.scale.toArray(),scale);
  }
});
test('reset restores initial placement after a completed transition', () => {
  const f=fixture(); f.adapter.apply({...initial,applied:true,fxActive:false}); f.adapter.apply(initial);
  assert.equal(f.covered.visible,true); assert.equal(f.raised.visible,false);
  assert.deepEqual(f.covered.position.toArray(),origin.map((x,i)=>x+i+1));
});
test('invalid foreign patch does not partially alter placement or visibility', () => {
  const f=fixture(); f.adapter.apply(initial); const before=[f.covered.position.toArray(),f.raised.position.toArray(),f.covered.visible,f.raised.visible];
  assert.throws(()=>f.adapter.apply({...initial,patch:'patch-004'}));
  assert.deepEqual([f.covered.position.toArray(),f.raised.position.toArray(),f.covered.visible,f.raised.visible],before);
});
test('disposal restores borrowed roots and rejects future updates', () => {
  const f=fixture(); f.adapter.apply({...initial,applied:true,fxActive:false}); f.adapter.dispose(); f.adapter.dispose();
  assert.deepEqual(f.covered.position.toArray(),[1,2,3]); assert.deepEqual(f.raised.position.toArray(),[1,2,3]);
  assert.equal(f.covered.visible,true); assert.equal(f.raised.visible,true);
  assert.throws(()=>f.adapter.apply(initial),/disposed/);
});
