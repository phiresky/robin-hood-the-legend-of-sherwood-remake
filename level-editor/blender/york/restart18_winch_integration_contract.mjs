/** Freeze the private visual handoff against native frames and the saved motion.
 * This command cannot publish assets or grant pending review approvals.
 */
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync, mkdirSync, writeFileSync, existsSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { bindWinchState } from './restart18_winch_state_binding.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../../..');
const base = resolve(root, 'level-editor/work/york-refinement');
const motionDir = resolve(base, 'restart2/winch-supported-motion-v1');
const sourceDir = resolve(base, 'geometry-pass-01/native-state-source-v1');
const output = resolve(base, 'restart2/winch-integration-contract-v1/contract.json');
if (existsSync(output)) throw new Error(`Refusing to overwrite ${output}`);
const pins = [];
function readPinned(path, expected) {
  const bytes = readFileSync(path);
  const sha256 = createHash('sha256').update(bytes).digest('hex');
  if (expected) assert.equal(sha256, expected, path);
  pins.push({ path, sha256, bytes: bytes.length });
  return bytes;
}
const plan = JSON.parse(readPinned(resolve(motionDir, 'state-plan/integration-plan-v1.json'),
  '7a09120350eb253a94b9c6bfc32962009cb3d0db8414b7a01294c3933c6de955'));
const source = JSON.parse(readPinned(resolve(sourceDir, 'manifest.json'), plan.source_manifest_sha256));
const record = source.records.find(r => r.id === 'patch-004');
const room = source.records.find(r => r.id === 'patch-005');
assert.ok(record && room);
assert.deepEqual(record.record, plan.source_patch_record);
const initial = record.rows.find(r => r.action === 'PatchInitial');
const transition = record.rows.find(r => r.action === 'PatchTransition');
assert.equal(initial.frames.length, 1);
assert.equal(transition.frames.length, 45);
assert.equal(record.rows.some(r => r.action === 'PatchFinal'), false);
assert.equal(room.record.transition_animation_valid, false);
for (const row of [initial, transition]) {
  for (const frame of row.frames) readPinned(resolve(sourceDir, frame.image), frame.sha256);
}
readPinned(resolve(motionDir, 'model.blend'), plan.model_sha256);
const motion = JSON.parse(readPinned(resolve(motionDir, 'motion.json'),
  '8d1cd9f0f0199893a18cdc24341dc3309cdc52a42fc6edbcdc21ff7f6210a380'));
assert.equal(motion.model_sha256, plan.model_sha256);
assert.equal(motion.rows.length, 45);
for (let n = 0; n < 45; n++) {
  assert.equal(motion.rows[n].frame, n);
  assert.equal(motion.rows[n].tick, 2 * n);
  assert.equal(transition.frames[n].delay, 1);
  assert.equal(transition.frames[n].sound_id, n === 0 ? 360 : 0);
}
// Independently step the inclusive frame delay and 16-bit reset counter.
// The binding itself neither owns nor advances this test clock.
let frame = 0, counter = 65535, terminated = false;
const trace = [];
for (let update = 0; update <= 90; update++) {
  if (update > 0) {
    counter = (counter + 1) & 65535;
    if (counter > transition.frames[frame].delay) { counter = 0; frame++; }
    assert.ok(frame < 45);
    terminated = frame === 44 && counter === transition.frames[frame].delay;
  }
  const snapshot = { patch: 'patch-004', applied: terminated, inTransition: !terminated,
    fxActive: !terminated, frame, counter, roomCoverApplied: false };
  const view = bindWinchState(snapshot);
  assert.equal(view.pose, plan.native_tick_trace[update].frame);
  assert.equal(counter, plan.native_tick_trace[update].counter);
  assert.equal(terminated, plan.native_tick_trace[update].terminated);
  trace.push({ update, snapshot, presentation: view });
}
const contract = {
  status: 'PRIVATE_SOURCE_BOUND_INTEGRATION_HANDOFF_PENDING_APPROVAL_AND_RUNTIME_WIRING',
  model: { path: resolve(motionDir, 'model.blend'), sha256: plan.model_sha256 },
  patch: 'patch-004', roomCoverPatch: 'patch-005',
  scope: 'Winch visual state binding; room cover remains an independently composed owner',
  review: { stableGeometry: 'Pending V23', chainHardwareAndMotion: 'Pending next collection',
    texture: 'Not authorized by a geometry approval yet', publication: 'Not permitted by this handoff' },
  stateProvider: { timing: 'Read after the authoritative native update, including endpoint application',
    required: ['applied', 'inTransition', 'fxActive', 'frame', 'counter', 'roomCoverApplied'],
    snapshot: 'Use a consistent snapshot for patch004 and patch005; do not derive state from selection or wall-clock time' },
  animation: { poses: 45, sampleTick: '2 * native frame', interpolation: 'STEP',
    initial: 'Empty', applied: 'Pose44 persists, dynamic FX inactive',
    restart: 'Commands remain with native patch handler; repeated mid-transition apply completes, not restarts' },
  audio: { owner: 'Existing native FX presentation event path', addedEmitters: 0,
    sourceSound: 360, sourceFrame: 0, firstEligibleUpdate: 1,
    caveat: 'Eligibility depends on active/displayed state; do not synthesize a missed presentation event during seek or reload' },
  unresolved: ['Live state provider hookup', 'Actual exported clip identity/timebase',
    'Original-camera background and independent room-cover composition',
    'Live pause/seek/save-load/selection testing', 'Approved texture fill and guarded publication'],
  pins, trace,
};
mkdirSync(dirname(output), { recursive: true });
writeFileSync(output, JSON.stringify(contract, null, 2) + '\n', { flag: 'wx' });
console.log(JSON.stringify({ output, resources: pins.length, nativeUpdates: trace.length,
  status: contract.status }));
