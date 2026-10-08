import polygonClipping from "polygon-clipping";
import type { AssetWalkableSurface } from "./asset-gameplay.ts";
import type { PlacedJumpSegment } from "./assemble-jump-segments.ts";
import { mergeIntervals, type JumpEdge, type Interval } from "./jump-clearance.ts";
import type { Point } from "./level.ts";
import type { Vec3 } from "./scene.ts";
import { planeHeight, type HeightPlane } from "./gameplay-plane.ts";
import { NAVIGATION_HALF_DIAGONAL as half } from "./navigation-footprint.ts";

export interface JumpLandingBand {
  plane: HeightPlane;
  depth: number;
  helperNeeded?: boolean;
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
    let [a, b] = edge;
    if (Math.hypot(b[0] - a[0], b[1] - a[1]) < rules.minOverlap) continue;
    if (Math.abs(planeHeight(plane, a) - planeHeight(plane, b)) > 1e-4) {
      const gradientSquared = plane[0] ** 2 + plane[1] ** 2;
      const halfRise = (planeHeight(plane, b) - planeHeight(plane, a)) / 2;
      const adjustment = Math.abs(halfRise) / Math.sqrt(gradientSquared);
      if (adjustment > (rules.maxLevelAdjustment ?? 0)) {
        warnings.push(
          `Surface ${id} edge ${index}: sloped takeoff line omitted from automatic jumps.`,
        );
        continue;
      }
      // Align with the surface's level contour without changing its height plane.
      const offset: Point = [
        (plane[0] * halfRise) / gradientSquared,
        (plane[1] * halfRise) / gradientSquared,
      ];
      a = [a[0] + offset[0], a[1] + offset[1]];
      b = [b[0] - offset[0], b[1] - offset[1]];
    }
    const dx = b[0] - a[0],
      dy = b[1] - a[1],
      length = Math.hypot(dx, dy);
    if (length < rules.minOverlap) continue;
    const inward: Point = [dy / length, -dx / length];
    // Reserve the same footprint as the prepared graph, plus one unit for
    // final grid rounding, in both the normal and tangential directions.
    const normalClearance = half[0] * Math.abs(inward[0]) + half[1] * Math.abs(inward[1]);
    const alongClearance = (half[0] * Math.abs(dx) + half[1] * Math.abs(dy)) / length + 1;
    const inset = Math.max(rules.inset, normalClearance + 1);
    const at = (t: number, depth: number): Point => [
      a[0] + dx * t + inward[0] * depth,
      a[1] + dy * t + inward[1] * depth,
    ];
    const strip = [
      at(0, inset - normalClearance),
      at(1, inset - normalClearance),
      at(1, inset + Math.max(rules.landingDepth, normalClearance)),
      at(0, inset + Math.max(rules.landingDepth, normalClearance)),
    ];
    const outside = polygonClipping.difference([strip], [polygon, ...holes]);
    const along = (p: Point) => ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / (length * length);
    const forbidden: Interval[] = outside.map((region) => {
      const values = region.flat().map(along);
      return [
        Math.max(0, Math.min(...values) - alongClearance / length),
        Math.min(1, Math.max(...values) + alongClearance / length),
      ];
    });
    // Keep endpoints away from outer corners and integer-grid uncertainty.
    const margin = Math.max(alongClearance, rules.clearance?.radius ?? 0) / length;
    const blocked = mergeIntervals([[0, margin], ...forbidden, [1 - margin, 1]]);
    let start = 0,
      serial = 0;
    for (const [low, high] of blocked) {
      if ((low - start) * length >= rules.minOverlap) {
        const zone = `${id}/ledge-${index}-${serial++}`;
        segments.push({
          id: zone,
          long: rules.long ?? true,
          attachment: rules,
          surfaceInset: inset,
          edge: { zone, a: toWorld(at(start, inset)), b: toWorld(at(low, inset)) },
        });
        landings.set(zone, { plane, depth: rules.landingDepth, helperNeeded: rules.helperNeeded });
      }
      start = Math.max(start, high);
    }
    if (!serial)
      warnings.push(`Surface ${id} edge ${index}: no character-sized receiving span remains.`);
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
    helper: band.helperNeeded ?? false,
    anchor: [anchor[0], anchor[1] + z, z] as Vec3,
    polygon: [
      a,
      b,
      [b[0] + offset[0], b[1] + offset[1]],
      [a[0] + offset[0], a[1] + offset[1]],
    ] as Point[],
  };
}
