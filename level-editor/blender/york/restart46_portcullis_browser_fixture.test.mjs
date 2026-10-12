import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import {createRequire} from 'node:module';
import {runPortcullisBrowserFixture} from './restart46_portcullis_browser_fixture.mjs';
const require=createRequire(new URL('../../app/package.json',import.meta.url));
const THREE=require('three');
const base=new URL('../../work/active/york/restart2/',import.meta.url);
const read=path=>JSON.parse(fs.readFileSync(new URL(path,base)));
const approvedMotion=read('jamb-clearance-candidate-v1/motion-proposal.json');
const sourceRecord=read('../geometry-pass-01/native-state-source-v1/manifest.json').records.find(r=>r.id==='patch-000');
const contextViews=read('restart44-approved-portcullis-export-v1/covered/browser-request.json').context.views;
function fixture() {
  const covered=new THREE.Group(),raised=new THREE.Group(),gatehouse=new THREE.Group();
  for (const root of [covered,raised,gatehouse]) {
    const child=new THREE.Mesh(new THREE.BoxGeometry(),new THREE.MeshBasicMaterial());
    child.rotation.x=-Math.PI/2;root.add(child);
  }
  gatehouse.position.set(2378,109.8709487915039,1830);
  return {covered,raised,gatehouse,approvedMotion,sourceRecord,contextViews,
    originYUp:[2357.217041015625,109.8709487915039,1801.3960571289062]};
}
test('all45poses/92updates and18ordered captures; callback observes loaded scene state',async()=>{
  const f=fixture(), captures=[];
  const result=await runPortcullisBrowserFixture({...f,capture:async c=>{
    captures.push(c.label); assert.equal(c.roots.gatehouse,f.gatehouse);
    assert.equal(f.covered.visible,c.phase!=='applied');
    return {kind:'CPU_TEST_NO_IMAGE',label:c.label};
  }});
  assert.equal(result.poses,45);assert.equal(result.updates.length,92);assert.equal(result.captures,18);
  assert.equal(captures[0],'initial-view-0');
  assert.ok(captures.every((label,i)=>label.endsWith(i%2===0?'view-0':'view-2')));
  assert.equal(result.updates[88].transient_visible,true);
  assert.equal(result.updates[89].transient_visible,false);
  assert.equal(result.updates[89].retained_background,true);
  assert.deepEqual(f.covered.position.toArray(),[0,0,0]);
  assert.deepEqual(f.raised.position.toArray(),[0,0,0]);
});
test('reject already placed endpoint before callbacks or scene mutation',async()=>{
  const f=fixture();f.raised.position.y=69;
  await assert.rejects(runPortcullisBrowserFixture({...f,capture:()=>{throw Error('not called');}}),/unplaced/);
  assert.equal(f.raised.position.y,69);
});
test('capture failure restores borrowed endpoints',async()=>{
  const f=fixture();
  await assert.rejects(runPortcullisBrowserFixture({...f,capture:()=>{throw Error('capture failed');}}),/capture failed/);
  assert.deepEqual(f.covered.position.toArray(),[0,0,0]);
  assert.deepEqual(f.raised.position.toArray(),[0,0,0]);
});
