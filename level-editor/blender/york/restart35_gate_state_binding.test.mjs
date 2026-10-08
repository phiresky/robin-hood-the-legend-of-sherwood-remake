import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { createGateStateBinding } from './restart35_gate_state_binding.mjs';
const path = new URL('../../work/york-refinement/restart2/jamb-clearance-candidate-v1/motion-proposal.json', import.meta.url);
const bytes = readFileSync(path);
assert.equal(createHash('sha256').update(bytes).digest('hex'), '07fd102ae0a508bf77ff10f6d52dda6c746723d3da7011b312b9164b7758a446');
const motion = JSON.parse(bytes), sample = createGateStateBinding(motion);
const initial = { patch: 'patch-000', applied: false, inTransition: false, fxActive: true };
test('initial grille stays visible and applied grille persists after FX stops', () => {
  assert.equal(sample(initial).pose, 0);
  assert.equal(sample(initial).visible, true);
  const end = sample({ ...initial, applied: true, fxActive: false });
  assert.equal(end.pose, 44); assert.equal(end.appearance, 'raised'); assert.equal(end.dynamic, false);
});
test('all native update boundaries retain inclusive two-update frame durations', () => {
  const reset = sample({ ...initial, inTransition: true, frame: 0, counter: 65535 });
  assert.equal(reset.pose, 0);
  for (let update = 0; update <= 90; update++) {
    const done = update === 90;
    const frame = Math.min(44, Math.floor(update / 2));
    const state = { ...initial, inTransition: !done, applied: done,
      fxActive: !done, frame, counter: update % 2 };
    const result = sample(state);
    assert.equal(result.pose, frame);
    assert.equal(result.liftWorldZ, motion.rows[frame].nominal_lift_world_z);
  }
});
test('hidden pose inference and supported small settle are preserved exactly', () => {
  const pose = f => sample({ ...initial, inTransition: true, frame: f, counter: 0 });
  assert.match(pose(36).inference, /not measured/);
  assert.equal(pose(35).liftWorldZ, pose(36).liftWorldZ);
  assert.ok(pose(38).liftWorldZ < pose(37).liftWorldZ);
  assert.equal(pose(39).liftWorldZ, pose(44).liftWorldZ);
});
test('rejects missing authoritative state and never accepts another patch clock', () => {
  assert.throws(() => sample({ ...initial, patch: 'patch-004' }));
  assert.throws(() => sample({ ...initial, inTransition: true, fxActive: false }));
  for (const frame of [-1, 45, 1.5]) assert.throws(() => sample({ ...initial, inTransition: true, frame, counter: 0 }));
  assert.throws(() => sample({ ...initial, inTransition: true, frame: 1, counter: 65535 }));
});
test('presentation sampling is pure and independent of winch/room updates', () => {
  const state = Object.freeze({ ...initial, inTransition: true, frame: 22, counter: 1 });
  const first = sample(state);
  assert.deepEqual(first, sample({ ...state, winchFrame: 44, roomCoverApplied: true }));
  assert.deepEqual(first, sample(state));
  assert.equal('audio' in first, false); assert.equal('doorActions' in first, false);
});
