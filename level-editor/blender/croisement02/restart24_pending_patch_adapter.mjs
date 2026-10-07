// Private source-state plumbing. Pending workers are never runtime asset bindings.
import {
  nativePatchBackgroundFrame,
  nativeTransientPatchFrame,
} from '../../shared/src/native-state-presentation.ts';

const phases = new Set(['initial', 'forward', 'applied', 'reverse']);
const durationOf = frames => frames.reduce((sum, frame) => sum + frame.delay + 1, 0);
const require = (test, message) => { if (!test) throw Error(message); };

export function sourceSnapshot(binding, patch, phase, tick) {
  require(binding.id === patch.id && binding.focus_patch_id === patch.id, 'Wrong patch binding');
  require(phases.has(phase) && Number.isSafeInteger(tick) && tick >= 0, 'Invalid source phase/tick');
  const duration = durationOf(patch.transition), terminal = Math.max(1, duration - 1);
  require(binding.terminal_tick === terminal && binding.transition_duration === duration, 'Timing binding changed');
  // Production selectors reject invalid reversal before any adapter state changes.
  const background = nativePatchBackgroundFrame(patch, phase, tick);
  const transient = nativeTransientPatchFrame(patch, phase, tick);
  const forward = phase === 'forward' && tick < terminal;
  const reverse = phase === 'reverse' && tick < duration;
  const inTransition = forward || reverse;
  const effective = phase === 'forward' ? (forward ? 'forward' : 'applied')
    : phase === 'reverse' ? (reverse ? 'reverse' : 'initial') : phase;
  const applied = effective === 'applied' || effective === 'reverse';
  const frames = effective === 'initial' ? patch.initial : effective === 'applied' ? patch.final : patch.transition;
  let localTick = phase === 'forward' && !forward ? tick - terminal
    : phase === 'reverse' && !reverse ? tick - duration : tick;
  if (effective === 'reverse') localTick = Math.max(0, localTick - 1);
  const ordered = effective === 'reverse' ? [...frames].reverse() : frames;
  const loop = effective === 'initial' ? patch.initial_loop : effective === 'applied' ? patch.final_loop : false;
  let counter = ordered.length ? (loop ? localTick % durationOf(ordered) : Math.min(localTick, durationOf(ordered) - 1)) : 0;
  let index = -1;
  for (let i = 0; i < ordered.length; i++) {
    if (counter <= ordered[i].delay) { index = effective === 'reverse' ? frames.length - 1 - i : i; break; }
    counter -= ordered[i].delay + 1;
  }
  require((index < 0 ? undefined : frames[index]) === transient, 'Frame/counter differs from production selection');
  const endpoint = inTransition ? { kind: 'unmodeled-transition', display: 'native-art-only' }
    : binding.endpoints[applied ? 'applied' : 'initial'];
  return {
    id: binding.id, mission: binding.mission, phase, tick, effective_phase: effective,
    patch_active: binding.initially_active && !(effective === 'applied' && binding.definitive),
    patch_applied: applied, in_transition: inTransition,
    sprite: { active: !!transient, phase: effective, frame_index: index, counter, resource: transient ?? null },
    background: { integrated: !!background, stamp: background ?? null,
      restore_original: patch.integrate_in_background && (phase === 'initial' || phase === 'reverse') },
    aperture: binding.aperture ? { site: binding.aperture.site, cap: applied ? 'open' : 'closed',
      reset_cap: 'closed', requires_current_receiver_validation: true } : null,
    physical_endpoint_intent: endpoint,
    physical_render_ready: false,
  };
}

export class PendingPatchController {
  constructor(bindings, patches) {
    require(bindings.length > 0 && new Set(bindings.map(b => b.id)).size === bindings.length, 'Duplicate or empty bindings');
    this.bindings = new Map(bindings.map(b => [b.id, structuredClone(b)]));
    this.patches = new Map([...patches].map(([id,p]) => [id, structuredClone(p)]));
    this.states = new Map(); this.mission = undefined;
    for (const binding of this.bindings.values()) {
      require(this.patches.has(binding.id), 'Missing native patch');
      sourceSnapshot(binding, this.patches.get(binding.id), 'initial', 0);
    }
  }
  selectMission(mission) {
    require([...this.bindings.values()].some(b => b.mission === mission), 'Unknown mission');
    this.mission = mission; this.states.clear();
  }
  setPhase(id, phase, tick = 0) {
    const binding = this.bindings.get(id);
    require(binding && binding.mission === this.mission, 'Patch outside selected mission');
    const snapshot = sourceSnapshot(binding, this.patches.get(id), phase, tick);
    this.states.set(id, { phase, tick });
    return snapshot;
  }
  snapshot(id) {
    const binding = this.bindings.get(id);
    require(binding && binding.mission === this.mission, 'Patch outside selected mission');
    const state = this.states.get(id) ?? { phase: 'initial', tick: 0 };
    return sourceSnapshot(binding, this.patches.get(id), state.phase, state.tick);
  }
  forceReset(id) { return this.setPhase(id, 'initial', 0); }
  resetMission() { this.states.clear(); }
  dispatchSource(presentation, id) {
    const snapshot = this.snapshot(id);
    require(presentation?.mission === snapshot.mission && typeof presentation.setPatchState === 'function', 'Native presentation mission mismatch');
    presentation.setPatchState(id, snapshot.phase, snapshot.tick);
    return snapshot;
  }
  physicalBinding() { throw Error('Pending geometry/appearance approval, GLB export and current receiver validation; no runtime binding'); }
  clear() { this.states.clear(); this.mission = undefined; }
}
