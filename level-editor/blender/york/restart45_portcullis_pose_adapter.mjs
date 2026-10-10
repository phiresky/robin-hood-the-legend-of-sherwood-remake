/** Private browser fixture adapter; production StateDelivery remains endpoint-only. */
import { createGateStateBinding } from './restart35_gate_state_binding.mjs';

export function createPortcullisPoseAdapter(covered, raised, approvedMotion, originYUp) {
  if (!covered?.isObject3D || !raised?.isObject3D || covered === raised ||
      !Array.isArray(originYUp) || originYUp.length !== 3 || !originYUp.every(Number.isFinite))
    throw new Error('Distinct loaded endpoint roots and a finite Y-up origin are required');
  const sample = createGateStateBinding(approvedMotion);
  const saved = [covered, raised].map(root => ({root, position: root.position.clone(), visible: root.visible}));
  const place = (entry, lift) => entry.root.position.copy(entry.position)
    .add({x: originYUp[0], y: originYUp[1] + lift, z: originYUp[2]});
  let disposed = false;
  return {
    apply(snapshot) {
      if (disposed) throw new Error('Portcullis fixture is disposed');
      // Validate the authoritative snapshot before making any visible change.
      const pose = sample(snapshot);
      place(saved[0], pose.liftWorldZ);
      // Raised geometry is already lifted within the common exported pivot.
      place(saved[1], 0);
      covered.visible = pose.appearance === 'covered';
      raised.visible = pose.appearance === 'raised';
      covered.updateMatrixWorld(true);
      raised.updateMatrixWorld(true);
      return pose;
    },
    dispose() {
      if (disposed) return;
      for (const entry of saved) {
        entry.root.position.copy(entry.position);
        entry.root.visible = entry.visible;
        entry.root.updateMatrixWorld(true);
      }
      disposed = true;
    },
  };
}
