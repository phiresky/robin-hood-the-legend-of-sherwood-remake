/** Private physical gate sampler. The authoritative patch owns timing and audio. */
export function createGateStateBinding(approvedMotion) {
  const rows = approvedMotion.rows;
  if (!Array.isArray(rows) || rows.length !== 45 || rows.some((r, i) =>
    r.frame !== i || !Number.isFinite(r.nominal_lift_world_z))) {
    throw new Error('Expected the exact approved 45-pose gate proposal');
  }
  const lifts = rows.map(r => r.nominal_lift_world_z);
  return snapshot => {
    if (!snapshot || snapshot.patch !== 'patch-000') throw new Error('Expected patch-000');
    for (const key of ['applied', 'inTransition', 'fxActive']) {
      if (typeof snapshot[key] !== 'boolean') throw new Error(`Missing boolean ${key}`);
    }
    // Definitive completion persists even after its transient sprite disappears.
    let frame, phase, appearance;
    if (snapshot.applied) {
      frame = 44; phase = 'applied'; appearance = 'raised';
    } else if (!snapshot.inTransition) {
      frame = 0; phase = 'initial'; appearance = 'covered';
    } else {
      if (!snapshot.fxActive) throw new Error('Active transition requires native FX');
      frame = snapshot.frame;
      if (!Number.isInteger(frame) || frame < 0 || frame > 44) throw new Error('Invalid gate frame');
      if (![0, 1, 65535].includes(snapshot.counter) ||
          (snapshot.counter === 65535 && frame !== 0)) throw new Error('Invalid gate counter');
      phase = 'transition'; appearance = 'covered';
    }
    return { phase, visible: true, pose: frame, clipTick: frame * 2,
      liftWorldZ: lifts[frame], appearance, dynamic: phase === 'transition',
      inference: frame === 36 ? 'Hidden pose: approved 57-pixel inference, not measured parity' : null };
  };
}
