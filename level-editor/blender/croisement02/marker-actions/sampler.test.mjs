import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { resolve } from 'node:path';
import { createMarkerAdapter, sampleRow } from './sampler.mjs';

const root = resolve(import.meta.dirname, '../../../..');
const work = resolve(root, 'level-editor/work/croisement02-refinement');
const read = path => JSON.parse(readFileSync(path, 'utf8'));
const profile = read(resolve(work, 'state-target-evidence/profiles/TG_BowTarget-02/native-profile.json'));
const dir = resolve(work, 'restart2-state/marker-source-export-v1');
const exported = read(resolve(dir, 'manifest.json'));
const adapter = createMarkerAdapter(profile, exported);
const row = profile.rows.find(r => r.action_id === 0);
const snapshot = { mission:'Emb05_FoB_MP', targetIndex:2, action:0, completedUpdates:1,
  mode:'default', active:true, ambiance:'Day' };

test('actual 20-phase marker timing and reset sentinel', () => {
  assert.equal(row.frames.length, 20);
  assert.ok(row.frames.every(f => f.delay === 2));
  assert.deepEqual(sampleRow(row, 0), {index:0, frameCount:65535, motion:'in-progress'});
  for (const [updates,index,count] of [[1,0,0],[3,0,2],[4,1,0],[58,19,0],[60,19,2],[61,0,0]]) {
    const got = sampleRow(row, updates); assert.equal(got.index,index); assert.equal(got.frameCount,count);
  }
  assert.equal(sampleRow(row,58).motion,'done');
  assert.equal(sampleRow(row,60).motion,'terminated');
  assert.equal(sampleRow(row,60,'cyclic').motion,'in-progress');
});
test('freeze-last stops on arrival, not after the final delay', () => {
  for (const n of [58,60,6000]) assert.deepEqual(sampleRow(row,n,'freeze-last'),{index:19,frameCount:0,motion:'done'});
  const blank = profile.rows.find(r => r.action_id === 210);
  assert.deepEqual(sampleRow(blank,600,'freeze-last'),{index:0,frameCount:65535,motion:'in-progress'});
});
test('45 current target identities and visual anchors reconcile to prior export', () => {
  assert.equal(exported.instances.length,45);
  for (const binding of exported.instances) {
    const { mission,target_index,target } = binding.instance;
    const current=read(resolve(root,`datadirs/fullgame_gog_hackable/Data/Levels/${mission}.rhm.json`)).targets[target_index];
    for (const key of ['filename','profile_name','action','position_x','position_y','position_z','obstacle_index','action_position_x','action_position_y'])
      assert.equal(current[key],target[key],`${mission}/${target_index}/${key}`);
    assert.ok(current.position_z>0);
    assert.equal(adapter.sample({...snapshot,mission,targetIndex:target_index}).targetIndex,target_index);
  }
});
test('all 66 source/body/shadow assets still match pinned export', () => {
  let count=0;
  for (const action of exported.actions) for (const frame of action.frames)
    for (const [file,hash] of [['raw_image','raw_sha256'],['body_image','body_sha256'],['shadow_mask','shadow_mask_sha256']]) {
      assert.equal(createHash('sha256').update(readFileSync(resolve(dir,frame[file]))).digest('hex'),frame[hash]);count++;
    }
  assert.equal(count,66);
});
test('hide actions and inactive snapshots clear both body and shadow; reset restores', () => {
  for (const action of [210,211]) {
    const got=adapter.sample({...snapshot,action,mode:'freeze-last',completedUpdates:20});
    assert.equal(got.action,action);assert.equal(got.body,null);assert.equal(got.shadow,null);assert.equal(got.visible,false);
  }
  const inactive=adapter.sample({...snapshot,active:false});assert.equal(inactive.body,null);assert.equal(inactive.shadow,null);
  assert.ok(adapter.sample({...snapshot,completedUpdates:0}).body);
});
test('phase switches body and shadow together with exact source offsets', () => {
  const first=adapter.sample(snapshot), next=adapter.sample({...snapshot,completedUpdates:13});
  assert.notEqual(first.body,next.body);assert.notEqual(first.shadow.image,next.shadow.image);
  assert.notDeepEqual(first.sourceOffset,next.sourceOffset);
  assert.equal(first.shadow.retention,0.6);
  assert.equal(adapter.sample({...snapshot,ambiance:'Fog'}).shadow.retention,0.9);
  assert.equal(first.shadow.operation,'destination-darken');assert.equal(first.shadow.reservedRgb565,31);
  assert.equal(first.soundEvent,null);assert.equal(first.placementBasis,'exported-canvas-bounds');
});
test('external eligible update count is the only time input; frozen caller snapshots are stable', () => {
  const first=adapter.sample(snapshot);
  for(let i=0;i<20;i++) assert.deepEqual(adapter.sample(snapshot),first);
  adapter.sample({...snapshot,targetIndex:3,completedUpdates:50});
  assert.deepEqual(adapter.sample(snapshot),first);
});
test('unsupported actions and corrupt data fail instead of idle or marker fallback', () => {
  for(const action of [1,160,undefined]) assert.throws(()=>adapter.sample({...snapshot,action}));
  for(const ticks of [-1,1.5,NaN]) assert.throws(()=>sampleRow(row,ticks));
  assert.throws(()=>sampleRow(row,1,'bored'));
  const broken=structuredClone(exported);broken.actions[0].frames[1].ticks++;
  assert.throws(()=>createMarkerAdapter(profile,broken));
});
