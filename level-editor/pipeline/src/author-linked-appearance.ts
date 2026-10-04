import type { AssetGameplay, GameplayAssetDescriptor } from "../../shared/src/asset-gameplay.ts";
import { validateAssetGameplay } from "../../shared/src/asset-gameplay.ts";
import type { Vec3 } from "../../shared/src/scene.ts";

/** Author an explicit shared control, preserving each asset's own state contents. */
export function authorLinkedAppearance(
  controller: GameplayAssetDescriptor,
  follower: GameplayAssetDescriptor,
  options: {
    transition: string;
    appearance: string;
    node: string;
    id: string;
    key: string;
    /** Convert controller transition-node coordinates into the follower node frame. */
    rebase: (point: Vec3) => Vec3;
  },
): { controller: AssetGameplay; follower: AssetGameplay } {
  if (controller.id === follower.id) throw new Error("Shared appearance needs separate assets");
  if (!controller.gameplay || !follower.gameplay) throw new Error("Missing asset gameplay");
  const left = structuredClone(controller.gameplay),
    right = structuredClone(follower.gameplay);
  const controls = left.movementTransitions?.filter((t) => t.id === options.transition) ?? [];
  if (controls.length !== 1) throw new Error("Shared appearance needs one controller transition");
  if (
    !options.id ||
    !options.key ||
    !options.appearance ||
    follower.parts.filter((part) => part.node === options.node).length !== 1
  )
    throw new Error("Shared appearance needs a local identity and node");
  if (
    right.movementTransitions?.some(
      (t) => t.id === options.id || t.appearances?.includes(options.appearance),
    )
  )
    throw new Error("Follower appearance already has a control");
  const control = controls[0]!;
  if (control.join && control.join.key !== options.key)
    throw new Error("Controller already has a different shared control");
  control.join ??= { key: options.key, point: [...control.waypoint] };
  const rebase = (point: Vec3): Vec3 => {
    const result = options.rebase([...point]);
    if (result.length !== 3 || !result.every(Number.isFinite))
      throw new Error("Invalid shared appearance frame conversion");
    return [...result];
  };
  const waypoint = rebase(control.waypoint);
  const contour = (points: [number, number][]): [number, number][] =>
    points.map(([x, y]) => {
      const point = rebase([x, y, control.waypoint[2]]);
      if (Math.abs(point[2] - waypoint[2]) > 1e-4)
        throw new Error("Shared control contour must remain horizontal in the asset frame");
      return [point[0], point[1]];
    });
  (right.movementTransitions ??= []).push({
    id: options.id,
    node: options.node,
    appearances: [options.appearance],
    join: { key: options.key, point: rebase(control.join.point) },
    waypoint,
    ...(control.waypointAnchor ? { waypointAnchor: rebase(control.waypointAnchor) } : {}),
    ...(control.waypointReceiverSegment
      ? {
          waypointReceiverSegment: [
            rebase(control.waypointReceiverSegment[0]),
            rebase(control.waypointReceiverSegment[1]),
          ],
        }
      : {}),
    active: control.active,
    definitive: control.definitive,
    initial: [],
    applied: [],
    applyPolygon: contour(control.applyPolygon),
    noApplyPolygon: contour(control.noApplyPolygon),
  });
  validateAssetGameplay(left, controller);
  validateAssetGameplay(right, follower);
  return { controller: left, follower: right };
}
