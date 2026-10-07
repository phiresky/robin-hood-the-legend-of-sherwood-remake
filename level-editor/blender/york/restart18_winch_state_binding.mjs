/** Private York winch presentation adapter; not connected to the live catalog.
 * Consume authoritative state after its update. Never advance a second clock,
 * dispatch gameplay actions, or emit audio while sampling a visual pose.
 */
export function bindWinchState(snapshot) {
  if (!snapshot || snapshot.patch !== 'patch-004') {
    throw new Error('York winch requires patch-004 state');
  }
  for (const key of ['applied', 'inTransition', 'fxActive', 'roomCoverApplied']) {
    if (typeof snapshot[key] !== 'boolean') throw new Error(`Missing boolean ${key}`);
  }
  const roomCover = { patch: 'patch-005', applied: snapshot.roomCoverApplied };
  // A completed background-integrated endpoint persists after dynamic FX stops.
  // The definitive applied state also takes precedence over a stale transition bit.
  if (snapshot.applied) {
    return { phase: 'applied', visible: true, pose: 44, clipTick: 88,
      dynamic: false, roomCover };
  }
  if (!snapshot.inTransition) {
    return { phase: 'initial', visible: false, pose: null, clipTick: null,
      dynamic: false, roomCover };
  }
  if (!snapshot.fxActive) throw new Error('Transition has no active winch FX');
  if (!Number.isInteger(snapshot.frame) || snapshot.frame < 0 || snapshot.frame > 44) {
    throw new Error('Winch transition frame must be an integer in 0..44');
  }
  if (![0, 1, 65535].includes(snapshot.counter)
    || (snapshot.counter === 65535 && snapshot.frame !== 0)) {
    throw new Error('Invalid winch frame counter or reset sentinel');
  }
  return { phase: 'transition', visible: true, pose: snapshot.frame,
    clipTick: snapshot.frame * 2, dynamic: true, roomCover };
}
