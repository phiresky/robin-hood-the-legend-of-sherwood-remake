import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { bindWinchState } from './restart18_winch_state_binding.mjs';

const planBytes = readFileSync(new URL('../../work/york-refinement/restart2/winch-supported-motion-v1/state-plan/integration-plan-v1.json', import.meta.url));
assert.equal(createHash('sha256').update(planBytes).digest('hex'),
  '7a09120350eb253a94b9c6bfc32962009cb3d0db8414b7a01294c3933c6de955');
const plan = JSON.parse(planBytes);
const initial = { patch: 'patch-004', applied: false, inTransition: false,
  fxActive: true, roomCoverApplied: false };

test('empty initial visual differs from transition frame zero under either room cover', () => {
  for (const roomCoverApplied of [false, true]) {
    const state = { ...initial, roomCoverApplied };
    assert.equal(bindWinchState(state).visible, false);
    assert.equal(bindWinchState(state).pose, null);
    const started = bindWinchState({ ...state, inTransition: true, frame: 0, counter: 65535 });
    assert.equal(started.visible, true);
    assert.equal(started.pose, 0);
    assert.equal(started.roomCover.applied, roomCoverApplied);
  }
});

test('all 91 recorded update boundaries bind to their exact native pose', () => {
  const seen = new Set();
  for (const row of plan.native_tick_trace) {
    const view = bindWinchState({ ...initial, inTransition: !row.terminated,
      applied: row.terminated, fxActive: !row.terminated,
      frame: row.frame, counter: row.counter });
    assert.equal(view.pose, row.frame, `update ${row.elapsed_updates}`);
    assert.equal(view.clipTick, row.clip_pose_tick);
    assert.equal(view.dynamic, !row.terminated);
    seen.add(view.pose);
  }
  assert.equal(seen.size, 45);
  assert.equal(plan.native_tick_trace.length, 91);
});

test('applied final pose survives stopped FX, invalid final row, reload and repeated reads', () => {
  const applied = { ...initial, applied: true, fxActive: false };
  const expected = bindWinchState(applied);
  assert.equal(expected.pose, 44);
  assert.equal(expected.dynamic, false);
  assert.deepEqual(bindWinchState(JSON.parse(JSON.stringify(applied))), expected);
  assert.deepEqual(bindWinchState(applied), expected);
  assert.deepEqual(bindWinchState({ ...applied, inTransition: true }), expected);
  assert.equal(bindWinchState(initial).visible, false);
});

test('pause, discontinuous seek and reload sample authoritative frame without accumulating time', () => {
  for (const frame of [0, 20, 4, 44, 1]) {
    const state = Object.freeze({ ...initial, inTransition: true, frame, counter: 0 });
    const first = bindWinchState(state);
    assert.equal(first.pose, frame);
    assert.deepEqual(bindWinchState(state), first);
    assert.deepEqual(bindWinchState(JSON.parse(JSON.stringify(state))), first);
    assert.equal(Object.hasOwn(first, 'sound'), false);
    assert.equal(Object.hasOwn(first, 'events'), false);
  }
});

test('room cover stays independent for four endpoints and every transition pose', () => {
  for (const applied of [false, true]) {
    const a = bindWinchState({ ...initial, applied });
    const b = bindWinchState({ ...initial, applied, roomCoverApplied: true });
    assert.deepEqual({ ...a, roomCover: b.roomCover }, b);
  }
  for (let frame = 0; frame < 45; frame++) {
    const state = { ...initial, inTransition: true, frame, counter: 0 };
    const a = bindWinchState(state);
    const b = bindWinchState({ ...state, roomCoverApplied: true });
    assert.deepEqual({ ...a, roomCover: b.roomCover }, b);
  }
});

test('missing state and impossible transitioning samples fail explicitly', () => {
  for (const value of [null, {}, { ...initial, patch: 'patch-005' },
    { ...initial, roomCoverApplied: undefined },
    { ...initial, inTransition: true, fxActive: false },
    { ...initial, inTransition: true, frame: 45, counter: 0 },
    { ...initial, inTransition: true, frame: 1.5, counter: 0 },
    { ...initial, inTransition: true, frame: 1, counter: 65535 },
    { ...initial, inTransition: true, frame: 1, counter: 2 }]) {
    assert.throws(() => bindWinchState(value));
  }
});
