/** Compare the approved physical poses with the production native artwork clock. */
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { nativeTransientPatchFrame, nativePatchBackgroundFrame } from '../../shared/src/native-state-presentation.ts';
import { createGateStateBinding } from './restart35_gate_state_binding.mjs';

const root = path.resolve(import.meta.dirname, '../../..');
const base = path.join(root, 'level-editor/work/active/york/restart2');
const sha = p => createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const motionPath = path.join(base, 'jamb-clearance-candidate-v1/motion-proposal.json');
assert.equal(sha(motionPath), '07fd102ae0a508bf77ff10f6d52dda6c746723d3da7011b312b9164b7758a446');
const source = path.join(root, 'level-editor/work/active/york/geometry-pass-01/native-state-source-v1/manifest.json');
assert.equal(sha(source), '996e048c12c2326d059c6665ff70bda548937e2d06aa87c8d3f898eee0525cfe');
const sourceState = JSON.parse(fs.readFileSync(source)).records.find(r => r.id === 'patch-000');
assert.equal(sourceState.record.definitive, true);
assert.equal(sourceState.record.integrate_in_background, true);
assert.equal(sourceState.record.end_animation_valid, false);
assert.deepEqual(sourceState.record.door_indices, [13, 14]);
const transition = sourceState.rows.find(r => r.action === 'PatchTransition').frames;
assert.equal(transition.length, 45);
assert.ok(transition.every(r => r.delay === 1));
for (const frame of transition) assert.equal(sha(path.join(path.dirname(source), frame.image)), frame.sha256);
const initial = sourceState.rows.find(r => r.action === 'PatchInitial').frames;
const state = {id: 'patch-000', definitive: true, integrate_in_background: true,
  activation: 'phases', initial, transition, final: [], initial_loop: true, final_loop: false};
const physical = createGateStateBinding(JSON.parse(fs.readFileSync(motionPath)));
const reset = {patch: 'patch-000', applied: false, inTransition: true, fxActive: true, frame: 0, counter: 65535};
assert.equal(physical(reset).pose, 0);
const checks = [];
for (let update = 1; update <= 92; update++) {
  // The reset sentinel precedes the first rendered tick; update1 is tick0.
  const tick = update - 1;
  const applied = update >= 90;
  const frame = Math.min(44, Math.floor(tick / 2));
  const result = physical({...reset, applied, inTransition: !applied, fxActive: !applied,
    frame, counter: tick % 2});
  const sprite = nativeTransientPatchFrame(state, 'forward', tick);
  const background = nativePatchBackgroundFrame(state, 'forward', tick);
  assert.equal(result.pose, frame);
  assert.equal(sprite, applied ? undefined : transition[frame]);
  assert.equal(background, applied ? transition[44] : undefined);
  checks.push({update, presentation_tick: tick, pose: result.pose,
    physical_visible: result.visible, transient_visible: !!sprite, retained_background: !!background});
}
assert.throws(() => nativeTransientPatchFrame(state, 'reverse', 0));
assert.equal(physical({...reset, inTransition: false, applied: false, fxActive: true}).pose, 0);
const output = path.join(base, 'restart44-gate-clock-audit-v1');
fs.mkdirSync(output);
const report = {status: 'PASS_PRIVATE_CLOCK_COMPARISON', checks,
  reset_counter: 65535, completion_update: 90, completion_presentation_tick: 89,
  exact_native_frames: 45, mission_door_indices: [13, 14],
  pins: Object.fromEntries([source, motionPath,
    path.join(root, 'level-editor/shared/src/native-state-presentation.ts'),
    path.join(import.meta.dirname, 'restart35_gate_state_binding.mjs')].map(p => [p, sha(p)])),
  limits: ['No production physical adapter installed.', 'No door collision or audio execution proof.',
    'The physical sampler reads authoritative patch state and emits no door/audio events.',
    'Frame36 pose remains explicitly approved inference.']};
fs.writeFileSync(path.join(output, 'report.json'), JSON.stringify(report, null, 2) + '\n');
console.log(JSON.stringify({status: report.status, updates: checks.length, completion_update: 90, completion_tick: 89}));
