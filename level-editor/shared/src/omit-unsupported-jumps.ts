import type { JumpLine, JumpLinePair, JumpZone, Point, SightObstacle } from "./level.ts";
import { pointInGameplayPolygon } from "./navigation-anchor.ts";

const cross = (a: Point, b: Point) => a[0] * b[1] - a[1] * b[0];

function segmentTouches(a: Point, b: Point, face: Point[]): boolean {
  if (pointInGameplayPolygon(a, face, true) || pointInGameplayPolygon(b, face, true)) return true;
  const direction: Point = [b[0] - a[0], b[1] - a[1]];
  return face.some((p, i) => {
    const q = face[(i + 1) % face.length]!;
    const edge: Point = [q[0] - p[0], q[1] - p[1]];
    const denominator = cross(direction, edge);
    if (Math.abs(denominator) < 1e-12) return false;
    const offset: Point = [p[0] - a[0], p[1] - a[1]];
    const t = cross(offset, edge) / denominator;
    const u = cross(offset, direction) / denominator;
    return t >= 0 && t <= 1 && u >= 0 && u <= 1;
  });
}

/**
 * Omit jump pairs with an elevated line lying entirely off the receiving faces
 * of its own surface. Such a line supports neither takeoff nor landing after placement,
 * for example a roof rotated until its projection is edge-on. Partially
 * covered lines remain for the separate coverage audits.
 */
export function omitUnsupportedJumps(
  pairs: { pair: JumpLinePair; sourceIndex: number }[],
  zones: JumpZone[],
  sight: SightObstacle[],
  label: (sourceIndex: number) => string,
): { pairs: { pair: JumpLinePair; sourceIndex: number }[]; zones: JumpZone[]; warnings: string[] } {
  const faces = new Map<string, Point[][]>();
  for (const receiver of sight) {
    if (!Array.isArray(receiver.projection_area)) continue;
    const key = receiver.projection_area.join("/");
    const group = faces.get(key) ?? [];
    group.push(receiver.points.map((p): Point => [p.x, p.y - p.z_top]));
    faces.set(key, group);
  }
  const warnings: string[] = [];
  const supported = (line: JumpLine, other: JumpLine) => {
    // Zero-height lines may use the implicit ground plane.
    if (line.point_a[2] === 0 && line.point_b[2] === 0) return true;
    // A line's own surface is the landing zone of the opposite line.
    const zone = zones[other.jump_zone_index]!;
    const receivers = faces.get(`${zone.sector}/${zone.layer}`);
    // Without receiving faces there is no surface to test against.
    if (!receivers) return true;
    const a: Point = [line.point_a[0], line.point_a[1]];
    const b: Point = [line.point_b[0], line.point_b[1]];
    return receivers.some((face) => segmentTouches(a, b, face));
  };
  const kept = pairs.filter(({ pair, sourceIndex }) => {
    const missing = [
      supported(pair.line1, pair.line2) ? undefined : 1,
      supported(pair.line2, pair.line1) ? undefined : 2,
    ].filter((side) => side !== undefined);
    if (!missing.length) return true;
    warnings.push(
      `Jump ${label(sourceIndex)}: line ${missing.join(" and ")} lies entirely outside its receiving surface after placement; connection omitted. Adjust the placement or the asset-local jump edge.`,
    );
    return false;
  });
  if (kept.length === pairs.length) return { pairs, zones, warnings };
  const used = [
    ...new Set(
      kept.flatMap(({ pair }) => [pair.line1.jump_zone_index, pair.line2.jump_zone_index]),
    ),
  ].sort((a, b) => a - b);
  return {
    pairs: kept.map(({ pair, sourceIndex }) => ({
      sourceIndex,
      pair: {
        ...pair,
        line1: { ...pair.line1, jump_zone_index: used.indexOf(pair.line1.jump_zone_index) },
        line2: { ...pair.line2, jump_zone_index: used.indexOf(pair.line2.jump_zone_index) },
      },
    })),
    zones: used.map((index) => zones[index]!),
    warnings,
  };
}
