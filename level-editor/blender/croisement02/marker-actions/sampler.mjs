/** Pure visual sampling. The caller owns simulation time and action resets. */
export function sampleRow(row, completedUpdates, mode = 'default') {
  if (!Number.isSafeInteger(completedUpdates) || completedUpdates < 0)
    throw new Error('completedUpdates must be a nonnegative integer');
  if (!['default', 'cyclic', 'freeze-last'].includes(mode))
    throw new Error(`Unsupported progression ${mode}`);
  if (!row.frames?.length || !Number.isInteger(row.action_done) ||
      row.action_done < 0 || row.action_done >= row.frames.length)
    throw new Error('Invalid row/action-done index');
  const durations = row.frames.map(frame => {
    if (!Number.isInteger(frame.delay) || frame.delay < 0 || frame.delay > 65534)
      throw new Error('Invalid frame delay');
    return frame.delay + 1;
  });
  let index = 0, count = 65535;
  if (completedUpdates > 0 && !(mode === 'freeze-last' && durations.length === 1)) {
    let ticks = completedUpdates - 1;
    if (mode !== 'freeze-last') ticks %= durations.reduce((a, b) => a + b, 0);
    while (index < durations.length - 1 && ticks >= durations[index]) {
      ticks -= durations[index++];
    }
    count = mode === 'freeze-last' && index === durations.length - 1 ? 0 : ticks;
  }
  const terminated = completedUpdates > 0 && mode === 'default' &&
    index === durations.length - 1 && (count === durations[index] - 1 || durations[index] === 1);
  const done = completedUpdates > 0 && count === 0 && index === row.action_done;
  return Object.freeze({ index, frameCount: count,
    motion: terminated ? 'terminated' : done ? 'done' : 'in-progress' });
}

/** Bind only the verified Bow Target profile, retaining empty action identities. */
export function createMarkerAdapter(profile, exported) {
  if (profile.name !== 'Bow Target' || exported.native_hz !== 25)
    throw new Error('Unsupported marker source');
  const rows = new Map(profile.rows.map(row => [row.action_id, row]));
  const actions = new Map(exported.actions.map(row => [row.action_id, row]));
  for (const action of [0, 210, 211]) {
    const row = rows.get(action), images = actions.get(action);
    if (!row || !images || row.direction !== 0 || row.frames.length !== images.frames.length)
      throw new Error(`Missing or incompatible marker action ${action}`);
    for (const [i, frame] of row.frames.entries()) {
      const image = images.frames[i];
      if (image.index !== i || image.ticks !== frame.delay + 1 ||
          image.offset[0] !== frame.offset_x - profile.center_x ||
          image.offset[1] !== frame.offset_y - profile.center_y ||
          !image.body_image || !image.shadow_mask)
        throw new Error(`Incompatible marker frame ${action}/${i}`);
    }
  }
  return Object.freeze({
    sample(snapshot) {
      const { mission, targetIndex, action, completedUpdates, mode = 'default', active, ambiance } = snapshot;
      if (typeof mission !== 'string' || !mission || !Number.isInteger(targetIndex) || targetIndex < 0 ||
          typeof active !== 'boolean' || !['Day', 'Fog', 'Night', 'Attack', 'Custom1', 'Custom2', 'Custom3', 'Custom4'].includes(ambiance))
        throw new Error('Invalid marker identity/visibility/ambiance');
      const row = rows.get(action), images = actions.get(action);
      if (![0, 210, 211].includes(action) || !row || !images) throw new Error(`Unknown marker action ${action}`);
      const phase = sampleRow(row, completedUpdates, mode);
      const frame = images.frames[phase.index];
      const visible = active && action === 0;
      // Both resources and their common placement are one atomic presentation value.
      return Object.freeze({ mission, targetIndex, action, ...phase, active, visible,
        body: visible ? frame.body_image : null,
        shadow: visible && frame.shadow_pixels > 0 ? Object.freeze({ image: frame.shadow_mask,
          operation: 'destination-darken', retention: ambiance === 'Fog' ? 0.9 : 0.6,
          reservedRgb565: 0x001f }) : null,
        sourceOffset: Object.freeze([...frame.offset]), sourceSize: Object.freeze([...frame.size]),
        canvasBounds: Object.freeze([...exported.canvas_bounds]),
        // The precomposed body and mask use canvasBounds; sourceOffset is provenance,
        // not an additional translation to apply to those images.
        placementBasis: 'exported-canvas-bounds', soundEvent: null });
    },
  });
}
