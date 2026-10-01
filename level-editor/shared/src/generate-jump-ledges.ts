import polygonClipping from "polygon-clipping";
import type { AssetWalkableSurface } from "./asset-gameplay.ts";
import type { PlacedJumpSegment } from "./assemble-jump-segments.ts";
import { mergeIntervals, type JumpEdge, type Interval } from "./jump-clearance.ts";
import type { Point } from "./level.ts";
import type { Vec3 } from "./scene.ts";
import { planeHeight, type HeightPlane } from "./gameplay-plane.ts";

export interface JumpLandingBand {
  plane: HeightPlane;
  depth: number;
}

/** Build ledges from this placement's surface, independently of other assets or saved neighbours. */
export function generateJumpLedges(
  id: string,
  polygon: Point[],
  holes: Point[][],
  plane: HeightPlane,
  rules: NonNullable<AssetWalkableSurface["jump"]>,
) {
  const segments: PlacedJumpSegment[] = [];
  const landings = new Map<string, JumpLandingBand>();
  const warnings: string[] = [];
  const area = polygon.reduce((sum, p, i) => {
    const q = polygon[(i + 1) % polygon.length]!;
    return sum + p[0] * q[1] - q[0] * p[1];
  }, 0);
  const toWorld = (p: Point): Vec3 => {
    const z = planeHeight(plane, p);
    return [p[0], p[1] + z, z];
  };
  for (const index of rules.edges ?? polygon.map((_, i) => i)) {
    const edge = [polygon[index]!, polygon[(index + 1) % polygon.length]!] as [Point, Point];
    if (area > 0) edge.reverse();
    const [a, b] = edge;
    const dx = b[0] - a[0],
      dy = b[1] - a[1],
      length = Math.hypot(dx, dy);
    if (length < rules.minOverlap) continue;
    const inward: Point = [dy / length, -dx / length];
    const at = (t: number, depth: number): Point => [
      a[0] + dx * t + inward[0] * depth,
      a[1] + dy * t + inward[1] * depth,
    ];
    if (
      Math.abs(planeHeight(plane, at(0, rules.inset)) - planeHeight(plane, at(1, rules.inset))) >
      1e-4
    ) {
      warnings.push(
        `Surface ${id} edge ${index}: sloped takeoff line omitted from automatic jumps.`,
      );
      continue;
    }
    const strip = [
      at(0, rules.inset),
      at(1, rules.inset),
      at(1, rules.inset + rules.landingDepth),
      at(0, rules.inset + rules.landingDepth),
    ];
    const outside = polygonClipping.difference([strip], [polygon, ...holes]);
    const along = (p: Point) => ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / (length * length);
    const forbidden: Interval[] = outside.map((region) => {
      const values = region.flat().map(along);
      return [Math.max(0, Math.min(...values)), Math.min(1, Math.max(...values))];
    });
    // Keep endpoints away from outer corners and integer-grid uncertainty.
    const margin = Math.max(1, rules.clearance?.radius ?? 0) / length;
    const blocked = mergeIntervals([[0, margin], ...forbidden, [1 - margin, 1]]);
    let start = 0,
      serial = 0;
    for (const [low, high] of blocked) {
      if ((low - start) * length >= rules.minOverlap) {
        const zone = `${id}/ledge-${index}-${serial++}`;
        segments.push({
          id: zone,
          long: true,
          attachment: rules,
          surfaceInset: rules.inset,
          edge: { zone, a: toWorld(at(start, rules.inset)), b: toWorld(at(low, rules.inset)) },
        });
        landings.set(zone, { plane, depth: rules.landingDepth });
      }
      start = Math.max(start, high);
    }
  }
  return { segments, landings, warnings };
}

/** Each retained flight span receives its own landing band and anchor. */
export function jumpLandingBand(id: string, edge: JumpEdge, band: JumpLandingBand) {
  const a: Point = [edge.a[0], edge.a[1] - edge.a[2]],
    b: Point = [edge.b[0], edge.b[1] - edge.b[2]];
  const dx = b[0] - a[0],
    dy = b[1] - a[1],
    length = Math.hypot(dx, dy);
  const offset: Point = [(dy / length) * band.depth, (-dx / length) * band.depth];
  const anchor: Point = [(a[0] + b[0] + offset[0]) / 2, (a[1] + b[1] + offset[1]) / 2];
  const z = planeHeight(band.plane, anchor);
  return {
    id,
    helper: false,
    anchor: [anchor[0], anchor[1] + z, z] as Vec3,
    polygon: [
      a,
      b,
      [b[0] + offset[0], b[1] + offset[1]],
      [a[0] + offset[0], a[1] + offset[1]],
    ] as Point[],
  };
}
