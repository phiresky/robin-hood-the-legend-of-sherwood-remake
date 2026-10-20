import polygonClipping from "polygon-clipping";
import type { JumpLinePair, JumpZone, Point, SightObstacle } from "./level.ts";

/** Check departure coverage, not receiver state availability, height continuity, or flight. */
export function createJumpDepartureSupportAudit(sight: SightObstacle[], zones: JumpZone[]) {
  const faces = new Map<string, Point[][]>();
  for (const receiver of sight) {
    if (!Array.isArray(receiver.projection_area)) continue;
    const key = receiver.projection_area.join("/");
    const group = faces.get(key) ?? [];
    group.push(receiver.points.map((point): Point => [point.x, point.y - point.z_top]));
    faces.set(key, group);
  }
  return (pair: JumpLinePair): string[] => {
    if (!pair.jump_long) return [];
    const lines = [pair.line1, pair.line2];
    return lines.flatMap((source, side) => {
      // Implicit ground can legitimately support a departure at height zero.
      if (source.point_a[2] <= 0 || source.point_b[2] <= 0) return [];
      const zone = zones[lines[1 - side]!.jump_zone_index]!;
      const receivers = faces.get(`${zone.sector}/${zone.layer}`) ?? [];
      const a: Point = [source.point_a[0], source.point_a[1]];
      const b: Point = [source.point_b[0], source.point_b[1]];
      const dx = b[0] - a[0],
        dy = b[1] - a[1];
      const length = Math.hypot(dx, dy);
      if (length === 0)
        return [`direction ${side + 1} takeoff support is unverified: collapsed jump edge`];
      const offset: Point = [(-15 * dy) / length, (15 * dx) / length];
      const strip: Point[] = [
        a,
        b,
        [b[0] + offset[0], b[1] + offset[1]],
        [a[0] + offset[0], a[1] + offset[1]],
      ];
      try {
        if (!polygonClipping.difference([strip], ...receivers.map((face) => [face])).length)
          return [];
        return [
          `direction ${side + 1} elevated takeoff animation lacks receiving coverage within its fifteen-unit departure strip; connection retained for review`,
        ];
      } catch (error) {
        if (!(error instanceof Error)) throw error;
        return [`direction ${side + 1} takeoff support is unverified: ${error.message}`];
      }
    });
  };
}
