import polygonClipping, { type MultiPolygon } from "polygon-clipping";
import earcut, { flatten } from "earcut";
import type { JumpEdge } from "./jump-clearance.ts";
import { planeHeight, type HeightPlane } from "./gameplay-plane.ts";
import type { Point } from "./level.ts";
import type { Vec3 } from "./scene.ts";
import type { VerticalFlightRibbon } from "./vertical-jump-clearance.ts";
import { takeoffReceiverRegions } from "./takeoff-receiver-regions.ts";

export interface TakeoffReceiver {
  plane: HeightPlane;
  polygon: Point[];
  layer?: number;
  sector?: number;
  maximumZ?: number;
}

interface FlightContraction {
  target: Vec3;
  minimum: number;
  maximum: number;
  rounding: number;
}

/** Bound only the current order, contracting uncertainty after each earlier order. */
export function contractFlightRange(
  starts: number[],
  orders: FlightContraction[],
  targetBounds: (target: Vec3, index: number) => number[],
  normalMagnitude = 1,
): [number, number] {
  let before: [number, number] = [Math.min(...starts), Math.max(...starts)],
    after = before;
  for (const [index, order] of orders.entries()) {
    before = after;
    const targets = targetBounds(order.target, index);
    const lowTarget = Math.min(...targets),
      highTarget = Math.max(...targets);
    after = [
      Math.min(
        ...[order.minimum, order.maximum].map(
          (alpha) => before[0] + alpha * (lowTarget - before[0]),
        ),
      ) -
        order.rounding * normalMagnitude,
      Math.max(
        ...[order.minimum, order.maximum].map(
          (alpha) => before[1] + alpha * (highTarget - before[1]),
        ),
      ) +
        order.rounding * normalMagnitude,
    ];
  }
  return [Math.min(before[0], after[0]), Math.max(before[1], after[1])];
}

/** Branch on possible native frame counts instead of merging their eight-unit jumps. */
export function longFlightFamily(
  start: Vec3,
  targets: Vec3[],
  initialError: number,
  heightRange: [number, number] = [start[2] - initialError, start[2] + initialError],
  finalOrderSnaps = false,
) {
  const f = Math.fround;
  const rounding = Math.max(1, ...[start, ...targets].flat().map(Math.abs)) * 2 ** -20;
  const frames = (distance: number) => Math.trunc(Math.max(1, f(f(distance * 0.125) - 1)));
  let branches = [
    {
      position: start,
      error: initialError + rounding,
      minimumZ: heightRange[0],
      maximumZ: heightRange[1],
      convexPadding: rounding,
      contractions: [] as FlightContraction[] | undefined,
    },
  ];
  const segments: {
    a: Vec3;
    b: Vec3;
    padding: number;
    minimumZ: number;
    maximumZ: number;
    convexPadding: number;
    contractions?: FlightContraction[];
    snap?: boolean;
  }[] = [];
  for (const [targetIndex, target] of targets.entries()) {
    const next: typeof branches = [];
    for (const { position, error, minimumZ, maximumZ, convexPadding, contractions } of branches) {
      const delta = target.map((value, axis) => f(value - position[axis]!));
      const distance = f(
        Math.sqrt(f(f(f(delta[0]! ** 2) + f(delta[1]! ** 2)) + f(delta[2]! ** 2))),
      );
      const minimum = distance - error;
      if (minimum <= 0.0001)
        throw new Error("Takeoff flight contains an unresolved zero-length order");
      for (let count = frames(minimum); count <= frames(distance + error); count++) {
        // The terminating update replaces the final increment with the bound
        // landing position before the actor is observed or drawn.
        const steps = finalOrderSnaps && targetIndex === targets.length - 1 ? count - 1 : count;
        const increment = delta.map((value) => f(value * f(8 / distance)));
        let end = position.map(f) as Vec3;
        for (let step = 0; step < steps; step++)
          end = end.map((value, axis) => f(value + increment[axis]!)) as Vec3;
        const padding = Math.max(1, Math.abs(1 - (steps * 8) / minimum)) * error + rounding * steps;
        const bounded = steps * 8 <= minimum || (minimumZ === maximumZ && minimumZ === target[2]);
        const low = bounded ? Math.min(minimumZ, target[2]) : Math.min(minimumZ, end[2] - padding);
        const high = bounded ? Math.max(maximumZ, target[2]) : Math.max(maximumZ, end[2] + padding);
        const hullPadding = convexPadding + rounding * (steps + 1);
        const contracted =
          contractions && steps * 8 <= minimum
            ? [
                ...contractions,
                {
                  target,
                  minimum: (steps * 8) / (distance + error),
                  maximum: (steps * 8) / minimum,
                  rounding: rounding * (steps + 1),
                },
              ]
            : undefined;
        segments.push({
          a: position,
          b: end,
          padding,
          minimumZ: low,
          maximumZ: high,
          convexPadding: hullPadding,
          contractions: contracted,
        });
        next.push({
          position: end,
          error: padding,
          minimumZ: low,
          maximumZ: high,
          convexPadding: hullPadding,
          contractions: contracted,
        });
      }
    }
    if (next.length > 256)
      throw new Error("Takeoff flight has too many unresolved frame-count branches");
    branches = next;
  }
  if (targets.length)
    for (const branch of branches)
      segments.push({
        a: branch.position,
        b: targets.at(-1)!,
        snap: true,
        padding: branch.error,
        minimumZ: Math.min(branch.minimumZ, targets.at(-1)![2]),
        maximumZ: Math.max(branch.maximumZ, targets.at(-1)![2]),
        convexPadding: branch.convexPadding,
        contractions: branch.contractions && [
          ...branch.contractions,
          {
            target: targets.at(-1)!,
            minimum: 1,
            maximum: 1,
            rounding,
          },
        ],
      });
  return segments;
}

/** Cover every sprite takeoff distance from zero to fifteen units over the placed receivers. */
export function longTakeoffRibbons(
  source: JumpEdge,
  sourcePlane: HeightPlane,
  receivers: TakeoffReceiver[],
  goals: Vec3[][],
  topology?: { sector: number; layer: number },
  boundsPlanes: HeightPlane[] = [],
  motionPolygon?: Point[],
  assisted = false,
  landingPlane?: HeightPlane,
): VerticalFlightRibbon[] {
  const a: Point = [source.a[0], source.a[1] - source.a[2]];
  const delta: Point = [source.b[0] - source.a[0], source.b[1] - source.b[2] - a[1]];
  const length = Math.hypot(...delta);
  const normal: Point = [(-15 * delta[1]) / length, (15 * delta[0]) / length];
  const strip: Point[] = [
    a,
    [a[0] + delta[0], a[1] + delta[1]],
    [a[0] + delta[0] + normal[0], a[1] + delta[1] + normal[1]],
    [a[0] + normal[0], a[1] + normal[1]],
  ];
  const matching = receivers.filter((receiver) =>
    receiver.plane.every((value, axis) => Math.abs(value - sourcePlane[axis]!) < 0.0001),
  );
  const origins = topology
    ? []
    : matching.filter(
        (receiver) => polygonClipping.intersection([strip], [receiver.polygon]).length,
      );
  const topologies = new Set(
    topology
      ? [`${topology.sector}:${topology.layer}`]
      : origins.map((receiver) => `${receiver.sector}:${receiver.layer}`),
  );
  if (topologies.size > 1)
    throw new Error("Long-jump takeoff has ambiguous receiving sectors or layers");
  const regions: { polygons: MultiPolygon; plane: HeightPlane }[] = [];
  if (receivers.length === 0) {
    regions.push({ polygons: [[strip]], plane: sourcePlane });
  } else {
    // Motion keeps its sector/layer during takeoff. Within that topology the
    // receiver with the greatest stored top bound wins, even across a roof seam.
    const candidates = receivers
      .filter((receiver) => topologies.has(`${receiver.sector}:${receiver.layer}`))
      .sort(
        (a, b) =>
          (b.maximumZ ?? Math.max(...b.polygon.map((point) => planeHeight(b.plane, point)))) -
          (a.maximumZ ?? Math.max(...a.polygon.map((point) => planeHeight(a.plane, point)))),
      );
    regions.push(...takeoffReceiverRegions(source, sourcePlane, candidates, motionPolygon));
  }
  const ribbons: VerticalFlightRibbon[] = [];
  const parameter = (point: Point) =>
    Math.max(
      0,
      Math.min(1, ((point[0] - a[0]) * delta[0] + (point[1] - a[1]) * delta[1]) / length ** 2),
    );
  for (const { polygons, plane } of regions) {
    const world = (point: Point): Vec3 => {
      const z = planeHeight(plane, point);
      return [point[0], point[1] + z, z + (assisted ? 40 : 0)];
    };
    const process = (triangle: Point[], depth = 0) => {
      const parameters = triangle.map(parameter);
      const vertices = triangle.map(world);
      const reduced = vertices.map((point, index): Vec3 => [
        point[0] - delta[0] * parameters[index]!,
        point[1] - delta[1] * parameters[index]!,
        point[2],
      ]);
      const center = [0, 1, 2].map(
        (axis) => reduced.reduce((sum, point) => sum + point[axis]!, 0) / reduced.length,
      ) as Vec3;
      const error = Math.max(
        ...reduced.map((point) => Math.hypot(...point.map((value, axis) => value - center[axis]!))),
      );
      if (error > 0.5) {
        if (depth >= 20) throw new Error("Long-jump takeoff subdivision did not converge");
        if (triangle.length === 4) {
          process([triangle[0]!, triangle[1]!, triangle[2]!], depth + 1);
          process([triangle[0]!, triangle[2]!, triangle[3]!], depth + 1);
          return;
        }
        const lengths = reduced.map((point, index) =>
          Math.hypot(...point.map((value, axis) => value - reduced[(index + 1) % 3]![axis]!)),
        );
        const first = lengths.indexOf(Math.max(...lengths)),
          second = (first + 1) % 3,
          third = (first + 2) % 3;
        const p = triangle[first]!,
          q = triangle[second]!;
        const middle: Point = [(p[0] + q[0]) / 2, (p[1] + q[1]) / 2];
        process([p, middle, triangle[third]!], depth + 1);
        process([middle, q, triangle[third]!], depth + 1);
        return;
      }
      ribbons.push({
        points: [vertices[0]!, vertices[1]!, vertices[2]!, vertices[3] ?? vertices[0]!],
        parameters: [
          parameters[0]!,
          parameters[1]!,
          parameters[2]!,
          parameters[3] ?? parameters[0]!,
        ],
        extraHeight: 0,
      });
      const low = Math.min(...parameters),
        high = Math.max(...parameters);
      if (assisted) {
        const first = vertices[parameters.indexOf(low)]!,
          last = vertices[parameters.indexOf(high)]!;
        ribbons.push({
          points: [
            [first[0], first[1], first[2] - 40],
            first,
            last,
            [last[0], last[1], last[2] - 40],
          ],
          parameters: [low, low, high, high],
          extraHeight: 0,
        });
      }
      const translated = (point: Vec3, t: number): Vec3 => [
        point[0] + delta[0] * t,
        point[1] + delta[1] * t,
        point[2],
      ];
      const heightRange: [number, number] = [
        Math.min(...vertices.map((point) => point[2])),
        Math.max(...vertices.map((point) => point[2])),
      ];
      const landing = (point: Vec3, t: number): Vec3 => {
        const world = translated(point, t);
        if (!landingPlane) return world;
        const map: Point = [world[0], world[1] - world[2]];
        const z = planeHeight(landingPlane, map);
        return [map[0], map[1] + z, z];
      };
      for (const targets of goals)
        for (const step of longFlightFamily(center, targets, error, heightRange, true)) {
          if (step.snap) {
            // Landing binds atomically after the final airborne update. Check
            // the occupied destination, without inventing a swept rebind path.
            const first = landing(step.b, low),
              last = landing(step.b, high);
            ribbons.push({
              points: [first, first, last, last],
              parameters: [low, low, high, high],
              extraHeight: 0,
              minimumZ: Math.min(first[2], last[2]),
              maximumZ: Math.max(first[2], last[2]),
              flightOrders: targets.length,
            });
            continue;
          }
          ribbons.push({
            points: [
              translated(step.a, low),
              translated(step.b, low),
              translated(step.b, high),
              translated(step.a, high),
            ],
            parameters: [low, low, high, high],
            extraHeight: 0,
            padding: step.padding,
            paddingAxes: [0, 1, 2].map((axis) =>
              [...reduced, ...targets].every(
                (point) => Math.fround(point[axis]!) === Math.fround(center[axis]!),
              )
                ? 0
                : step.padding,
            ) as Vec3,
            minimumZ: step.minimumZ,
            maximumZ: step.maximumZ,
            planeBounds:
              step.contractions &&
              boundsPlanes.map((plane) => {
                const distance = (point: Vec3) =>
                  point[2] - planeHeight(plane, [point[0], point[1]]);
                const range = contractFlightRange(
                  vertices.map(distance),
                  step.contractions!,
                  (point) => [distance(translated(point, low)), distance(translated(point, high))],
                  1 + Math.abs(plane[0]) + Math.abs(plane[1]),
                );
                return { plane, minimum: range[0] };
              }),
            convexPadding: step.convexPadding,
            flightOrders: targets.length,
          });
        }
    };
    for (const polygon of polygons) {
      if (assisted) {
        const departure = polygon[0]!
          .filter(
            (point) =>
              Math.abs((point[0] - a[0]) * normal[0] + (point[1] - a[1]) * normal[1]) < 1e-7,
          )
          .sort((p, q) => parameter(p) - parameter(q));
        if (departure.length >= 2 && parameter(departure.at(-1)!) - parameter(departure[0]!) > 1e-9)
          process([departure[0]!, departure.at(-1)!, departure.at(-1)!]);
        continue;
      }
      // Rectangular support strips vary only along takeoff distance. Partition
      // that distance directly instead of repeatedly subdividing the full span.
      const ring = polygon[0]!;
      const local = ring.map((point) => [
        parameter(point),
        ((point[0] - a[0]) * normal[0] + (point[1] - a[1]) * normal[1]) / 225,
      ]);
      const minT = Math.min(...local.map((point) => point[0]!)),
        maxT = Math.max(...local.map((point) => point[0]!));
      const minS = Math.min(...local.map((point) => point[1]!)),
        maxS = Math.max(...local.map((point) => point[1]!));
      if (
        polygon.length === 1 &&
        local.length >= 4 &&
        maxS > minS &&
        maxT > minT &&
        [minT, maxT].every((t) =>
          [minS, maxS].every((s) =>
            local.some((point) => Math.abs(point[0]! - t) < 1e-8 && Math.abs(point[1]! - s) < 1e-8),
          ),
        ) &&
        local.every(
          ([t, s]) =>
            (Math.abs(t! - minT) < 1e-8 || Math.abs(t! - maxT) < 1e-8) &&
            (Math.abs(s! - minS) < 1e-8 || Math.abs(s! - maxS) < 1e-8),
        )
      ) {
        const point = (t: number, s: number): Point => [
          a[0] + delta[0] * t + normal[0] * s,
          a[1] + delta[1] * t + normal[1] * s,
        ];
        const count = Math.ceil((maxS - minS) * 30);
        for (let i = 0; i < count; i++) {
          const low = minS + ((maxS - minS) * i) / count,
            high = minS + ((maxS - minS) * (i + 1)) / count;
          process([point(minT, low), point(maxT, low), point(maxT, high), point(minT, high)]);
        }
        continue;
      }
      const flat = flatten(polygon),
        indices = earcut(flat.vertices, flat.holes, flat.dimensions);
      for (let i = 0; i < indices.length; i += 3)
        process(
          indices
            .slice(i, i + 3)
            .map((index): Point => [flat.vertices[index * 2]!, flat.vertices[index * 2 + 1]!]),
        );
    }
  }
  return ribbons;
}
