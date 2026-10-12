import type { JumpLinePair, JumpZone, Point, SightObstacle } from "./level.ts";
import { pointInGameplayPolygon } from "./navigation-anchor.ts";

/** Check the full segment against a union of receiving faces, including narrow gaps. */
function covered(a: Point, b: Point, faces: Point[][]): boolean {
  const inside = (point: Point) => faces.some((face) => pointInGameplayPolygon(point, face, true));
  if (!inside(a) || !inside(b)) return false;
  const dx = b[0] - a[0],
    dy = b[1] - a[1];
  const length = dx * dx + dy * dy;
  if (!length) return true;
  const cuts = [0, 1];
  for (const face of faces)
    for (const [i, p] of face.entries()) {
      const q = face[(i + 1) % face.length]!;
      const ex = q[0] - p[0],
        ey = q[1] - p[1];
      const px = p[0] - a[0],
        py = p[1] - a[1];
      const determinant = dx * ey - dy * ex;
      if (Math.abs(determinant) > 1e-10) {
        const t = (px * ey - py * ex) / determinant;
        const u = (px * dy - py * dx) / determinant;
        if (t > 0 && t < 1 && u >= 0 && u <= 1) cuts.push(t);
      } else if (Math.abs(px * dy - py * dx) < 1e-8) {
        for (const point of [p, q]) {
          const t = ((point[0] - a[0]) * dx + (point[1] - a[1]) * dy) / length;
          if (t > 0 && t < 1) cuts.push(t);
        }
      }
    }
  cuts.sort((x, y) => x - y);
  return cuts.slice(1).every((end, i) => {
    const t = (cuts[i]! + end) / 2;
    return inside([a[0] + dx * t, a[1] + dy * t]);
  });
}

/** Prepare once per export; this checks coverage, not flight or state availability. */
export function createJumpReceiverSupportAudit(sight: SightObstacle[], zones: JumpZone[]) {
  const faces = new Map<string, Point[][]>();
  for (const receiver of sight) {
    if (!Array.isArray(receiver.projection_area)) continue;
    const key = receiver.projection_area.join("/");
    const group = faces.get(key) ?? [];
    group.push(receiver.points.map((p): Point => [p.x, p.y - p.z_top]));
    faces.set(key, group);
  }
  return (pair: JumpLinePair): string[] => {
    const lines = [pair.line1, pair.line2];
    return lines.flatMap((source, side) => {
      const destination = lines[1 - side]!;
      const zone = zones[source.jump_zone_index]!;
      // Runtime transfers distance along the source vector from destination B.
      // This can differ from destination A when authored spans are unequal.
      const a: Point = [destination.point_b[0], destination.point_b[1]];
      const b: Point = [
        a[0] + source.point_b[0] - source.point_a[0],
        a[1] + source.point_b[1] - source.point_a[1],
      ];
      const receivers = faces.get(`${zone.sector}/${zone.layer}`) ?? [];
      // Zero-height endpoints may intentionally use the implicit ground plane.
      if (destination.point_a[2] === 0 && destination.point_b[2] === 0) return [];
      return covered(a, b, receivers)
        ? []
        : [
            `direction ${side + 1} landing span lacks complete receiving coverage after movement-grid rounding; connection retained for review`,
          ];
    });
  };
}
