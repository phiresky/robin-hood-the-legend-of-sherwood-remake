import type { JumpEdge } from "./jump-clearance.ts";
import { planeHeight, type HeightPlane } from "./gameplay-plane.ts";
import type { Vec3 } from "./scene.ts";

export interface VerticalFlightRibbon {
  /** Start/end at parameter zero, then end/start at parameter one. */
  points: [Vec3, Vec3, Vec3, Vec3];
  extraHeight: number;
  padding?: number;
  paddingAxes?: Vec3;
  paddingFactors?: [number, number, number, number];
  minimumZ?: number;
  maximumZ?: number;
  /** Signed height bounds above solid caps, after each non-overshooting order. */
  planeBounds?: { plane: HeightPlane; minimum: number }[];
  convexPadding?: number;
  flightOrders?: number;
  parameters?: [number, number, number, number];
  shift?: Vec3;
}

/** Intersect the full four-point convex envelope, including non-coplanar ribbons. */
export function ribbonParameterRange(
  points: VerticalFlightRibbon["points"],
  planes: ((point: Vec3, index: number) => number)[],
  parameters: [number, number, number, number] = [0, 0, 1, 1],
): [number, number] | undefined {
  // Barycentric coordinates for vertices 1..3; vertex 0 has weight 1-sum.
  const constraints = [
    [1, 0, 0, 0],
    [0, 1, 0, 0],
    [0, 0, 1, 0],
    [-1, -1, -1, 1],
  ];
  for (const plane of planes) {
    const values = points.map(plane);
    if (Math.max(...values) < 0) return undefined;
    if (Math.min(...values) >= 0) continue;
    const row = [
      values[1]! - values[0]!,
      values[2]! - values[0]!,
      values[3]! - values[0]!,
      values[0]!,
    ];
    const scale = Math.max(...row.slice(0, 3).map(Math.abs));
    constraints.push(row.map((value) => value / scale));
  }
  const cross = (a: number[], b: number[]) => [
    a[1]! * b[2]! - a[2]! * b[1]!,
    a[2]! * b[0]! - a[0]! * b[2]!,
    a[0]! * b[1]! - a[1]! * b[0]!,
  ];
  const dot = (a: number[], b: number[]) => a[0]! * b[0]! + a[1]! * b[1]! + a[2]! * b[2]!;
  let low = Infinity,
    high = -Infinity;
  for (let i = 0; i < constraints.length; i++)
    for (let j = i + 1; j < constraints.length; j++)
      for (let k = j + 1; k < constraints.length; k++) {
        const a = constraints[i]!,
          b = constraints[j]!,
          c = constraints[k]!;
        const bc = cross(b, c),
          ca = cross(c, a),
          ab = cross(a, b);
        const determinant = dot(a, bc);
        if (Math.abs(determinant) < 1e-10) continue;
        const weights = [0, 1, 2].map(
          (axis) => (-a[3]! * bc[axis]! - b[3]! * ca[axis]! - c[3]! * ab[axis]!) / determinant,
        );
        if (constraints.some((row) => dot(row, weights) + row[3]! < -1e-8)) continue;
        const parameter = Math.max(
          0,
          Math.min(
            1,
            parameters[0] +
              weights.reduce(
                (sum, weight, index) => sum + weight * (parameters[index + 1]! - parameters[0]),
                0,
              ),
          ),
        );
        low = Math.min(low, parameter);
        high = Math.max(high, parameter);
      }
  return low <= high ? [low, high] : undefined;
}

/** Plane-aware envelopes for native climbing orders, prepared only during export. */
export function verticalFlightRibbons(
  source: JumpEdge,
  destination: JumpEdge,
  sourcePlane: HeightPlane,
  destinationPlane: HeightPlane,
): VerticalFlightRibbon[] {
  const ribbons: VerticalFlightRibbon[] = [];
  const bind = (point: Vec3, plane: HeightPlane): Vec3 => {
    const y = point[1] - point[2];
    const z = planeHeight(plane, [point[0], y]);
    return [point[0], y + z, z];
  };
  const lift = (point: Vec3, amount: number): Vec3 => [point[0], point[1], point[2] + amount];
  const add = (a: Vec3, b: Vec3, c: Vec3, d: Vec3, extraHeight = 0, shift?: Vec3) =>
    ribbons.push({ points: [a, b, c, d], extraHeight, shift });
  const airborne = (a: Vec3, b: Vec3, c: Vec3, d: Vec3, speed: number) => {
    // Delta varies affinely across the span. Its minimum norm bounds the
    // one-frame overshoot for every takeoff, without sampling the span.
    const first = b.map((value, axis) => value - a[axis]!);
    const difference = c.map((value, axis) => value - d[axis]! - first[axis]!);
    const squared = difference.reduce((sum, value) => sum + value * value, 0);
    const t =
      squared === 0
        ? 0
        : Math.max(
            0,
            Math.min(
              1,
              -first.reduce((sum, value, axis) => sum + value * difference[axis]!, 0) / squared,
            ),
          );
    const distance = Math.hypot(...first.map((value, axis) => value + t * difference[axis]!));
    if (distance < 1e-4)
      throw new Error("Climbing flight collapses to zero length within the span");
    const scale = Math.max(1, speed / distance);
    const extend = (start: Vec3, target: Vec3): Vec3 =>
      target.map((value, axis) => start[axis]! + (value - start[axis]!) * scale) as Vec3;
    add(a, extend(a, b), extend(d, c), d);
  };
  const a = bind(source.a, sourcePlane),
    d = bind(source.b, sourcePlane);
  const b = destination.b,
    c = destination.a;
  if (b[2] > source.a[2]) {
    const dx = source.b[0] - source.a[0],
      dy = source.b[1] - source.a[1];
    const length = Math.hypot(dx, dy);
    const target = (point: Vec3): Vec3 => [
      point[0] + (15 * dy) / length,
      point[1] - (15 * dx) / length,
      point[2] - 60,
    ];
    const lowB = target(b),
      lowC = target(c);
    for (const amount of [0, 40]) {
      add(a, lift(a, amount), lift(d, amount), d);
      airborne(lift(a, amount), lowB, lowC, lift(d, amount), 15);
    }
    const boundB = bind(lowB, destinationPlane),
      boundC = bind(lowC, destinationPlane);
    add(lowB, boundB, boundC, lowC);
    // The action-point lift changes Z alone. Subsequent plane-bound movement
    // can therefore start sixty map-Y units earlier than the pre-lift segment.
    // Reserve that translated segment and the full lift timing envelope.
    const dz = -60 * destinationPlane[1];
    add(boundB, bind(b, destinationPlane), bind(c, destinationPlane), boundC, 60, [
      0,
      -60 + dz,
      dz,
    ]);
  } else {
    add(a, lift(a, -50), lift(d, -50), d);
    airborne(lift(a, -50), b, c, lift(d, -50), 20);
    add(b, bind(b, destinationPlane), bind(c, destinationPlane), c);
  }
  return ribbons;
}
