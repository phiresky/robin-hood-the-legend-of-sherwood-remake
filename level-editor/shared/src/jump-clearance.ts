import earcut from "earcut";
import type { Point, SightObstacle } from "./level.ts";
import type { Vec3 } from "./scene.ts";
import { heightPlane, planeHeight, type HeightPlane } from "./gameplay-plane.ts";
import {
  ribbonParameterRange,
  verticalFlightRibbons,
  type VerticalFlightRibbon,
} from "./vertical-jump-clearance.ts";

export type JumpEdge = { zone: string; a: Vec3; b: Vec3 };
type Vertex = [number, number, number, number];
type Plane = (point: Vertex, body: JumpBody) => number;
export interface JumpBody {
  radius: number;
  height: number;
}
export type Interval = [number, number];
const EPSILON = 1e-4;

/** Runtime long-jump integration, including the takeoff point. World Y includes elevation. */
export function longJumpTrajectory(start: Vec3, destination: Vec3): Vec3[] {
  const f = Math.fround;
  const direction = destination.map((n, i) => f(n - start[i]!)) as Vec3;
  const gravity = f(f(-8.01) * f(0.7));
  // The character apex (0.5) gives two flight-time units with this gravity.
  const velocity = direction.map((n) => f(n * f(0.5 / 3))) as Vec3;
  velocity[2] = f(velocity[2] - f(gravity * 2));
  let position = start.map(f) as Vec3;
  const points = [position];
  for (let i = 0; i < 50; i++) {
    const nextVelocityZ = f(f(gravity * 2) + velocity[2]);
    if (position[2] < 0 && nextVelocityZ <= 0) break;
    const next = position.map((n, axis) => f(f(velocity[axis]! * 2) + n)) as Vec3;
    const delta = next.map((n, axis) => f(n - destination[axis]!));
    const dot = f(
      f(f(direction[0] * delta[0]!) + f(direction[1] * delta[1]!)) + f(direction[2] * delta[2]!),
    );
    if (dot > f(-0.1)) return [...points, destination];
    points.push(next);
    position = next;
    velocity[2] = nextVelocityZ;
  }
  throw new Error("Jump trajectory does not reach its landing edge");
}

function clip(vertices: Vertex[], plane: (point: Vertex) => number): Vertex[] {
  const output: Vertex[] = [];
  for (let i = 0; i < vertices.length; i++) {
    const a = vertices[i]!,
      b = vertices[(i + 1) % vertices.length]!;
    const da = plane(a),
      db = plane(b);
    if (da >= 0) output.push(a);
    if (da >= 0 !== db >= 0) {
      const t = da / (da - db);
      output.push(a.map((n, axis) => n + t * (b[axis]! - n)) as Vertex);
    }
  }
  return output;
}

/** Airborne orders retain their actual endpoint until the final landing snap. */
export function integratedLongJumpTrajectory(start: Vec3, targets: Vec3[]): Vec3[] {
  return integratedJumpTrajectory(start, targets, "long");
}

/** Fixed native airborne increments; the final order snaps to its destination. */
export function integratedJumpTrajectory(
  start: Vec3,
  targets: Vec3[],
  kind: "long" | "up" | "down",
): Vec3[] {
  const f = Math.fround;
  const speed = kind === "long" ? 8 : kind === "up" ? 15 : 20;
  const rate = f(kind === "long" ? 0.125 : kind === "up" ? 0.06666666666666667 : 0.05);
  let position = start.map(f) as Vec3;
  const path = [position];
  for (const target of targets) {
    const delta = target.map((n, axis) => f(n - position[axis]!));
    const distance = f(Math.sqrt(f(f(f(delta[0]! ** 2) + f(delta[1]! ** 2)) + f(delta[2]! ** 2))));
    if (!(distance > 0)) throw new Error("Jump flight has a zero-length airborne order");
    const increment = delta.map((n) => f(n * f(speed / distance)));
    const frames = Math.trunc(Math.max(1, f(f(distance * rate) - 1)));
    for (let frame = 0; frame < frames; frame++)
      position = position.map((n, axis) => f(n + increment[axis]!)) as Vec3;
    path.push(position);
  }
  if (targets.length) path.push(targets.at(-1)!);
  return path;
}

export function mergeIntervals(intervals: Interval[]): Interval[] {
  const result: Interval[] = [];
  for (const [a, b] of [...intervals].sort((a, b) => a[0] - b[0])) {
    const last = result.at(-1);
    if (last && a <= last[1] + EPSILON) last[1] = Math.max(last[1], b);
    else result.push([a, b]);
  }
  return result;
}

/** Bound changes in fixed-step integration caused by a varying launch height. */
export function boundedLongJumpTrajectory(start: Vec3, targets: Vec3[], error: number) {
  const positions = integratedLongJumpTrajectory(start, targets);
  const result: { a: Vec3; b: Vec3; padding: number }[] = [];
  const rounding = Math.max(1, ...[start, ...targets].flat().map(Math.abs)) * 2 ** -20;
  let previous = start;
  for (const [index, target] of targets.entries()) {
    const distance = Math.hypot(...target.map((value, axis) => value - previous[axis]!));
    const minimum = distance - error - rounding;
    if (minimum <= EPSILON) throw new Error("Long-flight uncertainty reaches a zero-length order");
    const frames = (length: number) =>
      Math.trunc(Math.max(1, Math.fround(Math.fround(length * 0.125) - 1)));
    const count = frames(distance);
    const differingFrames = Math.max(
      count - frames(minimum),
      frames(distance + error + rounding) - count,
    );
    // For a fixed frame count, radial motion preserves radial error and scales
    // tangential error by |1 - travelled/distance|. A frame-count change adds
    // at most one eight-unit step per differing frame.
    const scale = Math.max(1, Math.abs(1 - (count * 8) / minimum));
    error = scale * error + differingFrames * 8 + rounding;
    const next = positions[index + 1]!;
    result.push({ a: previous, b: next, padding: error });
    previous = next;
  }
  if (targets.length) result.push({ a: previous, b: targets.at(-1)!, padding: error });
  return result;
}

/** Precompute solid prisms once, then intersect the full flight ribbon, not sampled rays. */
export function createJumpClearance(obstacles: SightObstacle[]) {
  const receivers = obstacles
    .filter((shape) => shape.projection_area)
    .flatMap((shape) => {
      const anchors =
        shape.projection_plane ?? shape.points.slice(0, 3).map((p): Vec3 => [p.x, p.y, p.z_top]);
      const plane = heightPlane(anchors.map(([x, y, z]) => [x, y - z, z]));
      return [
        {
          plane,
          polygon: shape.points.map((p): Point => [p.x, p.y - p.z_top]),
          minX: Math.min(...shape.points.map((p) => p.x)),
          maxX: Math.max(...shape.points.map((p) => p.x)),
          minY: Math.min(...shape.points.map((p) => p.y - p.z_top)),
          maxY: Math.max(...shape.points.map((p) => p.y - p.z_top)),
        },
      ];
    });
  const receiverPlane = (edge: JumpEdge): HeightPlane => {
    const planes = [edge.a, edge.b, edge.a.map((n, i) => (n + edge.b[i]!) / 2) as Vec3].map(
      (point) => {
        const x = point[0],
          y = point[1] - point[2];
        const candidates = receivers.filter((r) => {
          if (
            x < r.minX ||
            x > r.maxX ||
            y < r.minY ||
            y > r.maxY ||
            Math.abs(planeHeight(r.plane, [x, y]) - point[2]) > 1 + EPSILON
          )
            return false;
          let inside = false;
          for (let i = 0; i < r.polygon.length; i++) {
            const a = r.polygon[i]!,
              b = r.polygon[(i + 1) % r.polygon.length]!;
            const cross = (x - a[0]) * (b[1] - a[1]) - (y - a[1]) * (b[0] - a[0]);
            if (
              Math.abs(cross) < EPSILON &&
              x >= Math.min(a[0], b[0]) &&
              x <= Math.max(a[0], b[0]) &&
              y >= Math.min(a[1], b[1]) &&
              y <= Math.max(a[1], b[1])
            )
              return true;
            if (a[1] > y !== b[1] > y && x < a[0] + ((y - a[1]) * (b[0] - a[0])) / (b[1] - a[1]))
              inside = !inside;
          }
          return inside;
        });
        const first: HeightPlane = candidates[0]?.plane ?? [0, 0, point[2]];
        if (
          candidates.some((r) =>
            r.plane.some((value, axis) => Math.abs(value - first[axis]!) > EPSILON),
          )
        )
          throw new Error("Climbing ledge has ambiguous receiving planes");
        return first;
      },
    );
    if (
      planes.some((plane) =>
        plane.some((value, axis) => Math.abs(value - planes[0]![axis]!) > EPSILON),
      )
    )
      throw new Error("Climbing ledge crosses different receiving planes; split the surface");
    return planes[0]!;
  };
  const prisms = obstacles
    .filter((shape) => shape.solid && shape.initial_active !== false)
    .flatMap((shape) => {
      const indices = earcut(shape.points.flatMap((p) => [p.x, p.y]));
      const result: { planes: Plane[]; bounds: [number, number, number, number] }[] = [];
      // Runtime caps use the first three vertices, even if later stored heights differ.
      const top = heightPlane(
        shape.projection_plane ?? shape.points.slice(0, 3).map((p) => [p.x, p.y, p.z_top]),
      );
      const bottom = heightPlane(
        shape.projection_plane ?? shape.points.slice(0, 3).map((p) => [p.x, p.y, p.z_bottom]),
      );
      for (let i = 0; i < indices.length; i += 3) {
        const points = indices.slice(i, i + 3).map((index) => shape.points[index]!);
        const area = points.reduce((sum, p, j) => {
          const q = points[(j + 1) % 3]!;
          return sum + p.x * q.y - q.x * p.y;
        }, 0);
        if (Math.abs(area) < EPSILON) continue;
        result.push({
          bounds: [
            Math.min(...points.map((p) => p.x)),
            Math.min(...points.map((p) => p.y)),
            Math.max(...points.map((p) => p.x)),
            Math.max(...points.map((p) => p.y)),
          ],
          planes: [
            ...points.map((p, j): Plane => {
              const q = points[(j + 1) % 3]!;
              return (v, body) =>
                Math.sign(area) * ((q.x - p.x) * (v[1] - p.y) - (q.y - p.y) * (v[0] - p.x)) +
                body.radius * (Math.abs(q.x - p.x) + Math.abs(q.y - p.y));
            }),
            (v, body) =>
              v[2] +
              body.height -
              planeHeight(bottom, [v[0], v[1]]) +
              body.radius * (Math.abs(bottom[0]) + Math.abs(bottom[1])) -
              EPSILON,
            (v, body) =>
              planeHeight(top, [v[0], v[1]]) -
              v[2] +
              body.radius * (Math.abs(top[0]) + Math.abs(top[1])) -
              EPSILON,
          ],
        });
      }
      return result;
    });
  return (
    edges: [JumpEdge, JumpEdge],
    long: boolean,
    body: JumpBody = { radius: 0, height: 0 },
  ): Interval[] => {
    if (edges.some((edge) => Math.abs(edge.a[2] - edge.b[2]) > EPSILON))
      throw new Error(
        "Sloped jump edges need an authored connection; automatic flight clearance requires level ledges",
      );
    const planes =
      !long && Math.abs(edges[0].a[2] - edges[1].a[2]) >= 60 ? edges.map(receiverPlane) : undefined;
    const sloped = planes?.some((plane) => Math.abs(plane[0]) + Math.abs(plane[1]) >= EPSILON);
    const blocked: Interval[] = [];
    for (const [source, destination, reverse] of [
      [edges[0], edges[1], false],
      [edges[1], edges[0], true],
    ] as const) {
      const dx = source.b[0] - source.a[0],
        dy = source.b[1] - source.a[1];
      const length = Math.hypot(dx, dy);
      if (length < EPSILON) throw new Error("Jump edge collapses on the movement grid");
      if (
        (-(destination.b[0] - source.a[0]) * dy +
          (destination.b[1] - destination.b[2] - source.a[1] + source.a[2]) * dx) /
          length <=
        15 + EPSILON
      )
        throw new Error("Ledges leave no forward flight after the 15-unit takeoff");
      const start: Vec3 = [
        source.a[0] - (15 * dy) / length,
        source.a[1] + (15 * dx) / length,
        source.a[2],
      ];
      const rise = destination.b[2] - source.a[2];
      const slopedRibbons = sloped
        ? verticalFlightRibbons(
            source,
            destination,
            planes![reverse ? 1 : 0]!,
            planes![reverse ? 0 : 1]!,
          )
        : [];
      const flights: [Vec3, Vec3, number, number?][] = [];
      const addPath = (path: Vec3[], extraHeight = 0) => {
        for (let i = 1; i < path.length; i++) flights.push([path[i - 1]!, path[i]!, extraHeight]);
      };
      // At intermediate heights, an upright actor climbs while an assisted actor
      // still uses the long-flight branch. Reserve both possible paths.
      if (long || Math.abs(rise) < 100) {
        const targets = longJumpTrajectory(start, destination.b).slice(1);
        const shoulders: Vec3 = [source.a[0], source.a[1], source.a[2] + 40];
        for (const path of [
          [source.a, start, ...targets],
          integratedLongJumpTrajectory(start, targets),
          integratedLongJumpTrajectory(start, [destination.b]),
          [source.a, ...integratedLongJumpTrajectory(shoulders, targets)],
        ])
          addPath(path);
        flights.push([start, destination.b, 0]);
        if (sloped) {
          const bind = (point: Vec3, plane: HeightPlane): Vec3 => {
            const y = point[1] - point[2];
            const z = planeHeight(plane, [point[0], y]);
            return [point[0], y + z, z];
          };
          const sourcePlane = planes![reverse ? 1 : 0]!;
          const boundSource = bind(source.a, sourcePlane);
          const boundStart = bind(start, sourcePlane);
          const boundShoulders: Vec3 = [boundSource[0], boundSource[1], boundSource[2] + 40];
          const endHeight = planeHeight(sourcePlane, [source.b[0], source.b[1] - source.b[2]]);
          const error = Math.SQRT2 * Math.abs(endHeight - boundSource[2]);
          // Takeoff stays bound to the source plane. Flight targets still use
          // the stored ledge height; only the actual integration start changes.
          // Bound the small receiver-height variation across rounded ledges.
          flights.push(
            [boundSource, boundStart, 0, error],
            [boundSource, boundShoulders, 0, error],
          );
          for (const [launch, goals] of [
            [boundStart, targets],
            [boundStart, [destination.b]],
            [boundShoulders, targets],
          ] satisfies [Vec3, Vec3[]][])
            for (const step of boundedLongJumpTrajectory(launch, goals, error))
              flights.push([step.a, step.b, 0, step.padding]);
          addPath([destination.b, bind(destination.b, planes![reverse ? 0 : 1]!)]);
        }
      }
      if (!sloped && !long && Math.abs(rise) >= 60) {
        if (rise > 0) {
          const target: Vec3 = [
            destination.b[0] + (15 * dy) / length,
            destination.b[1] - (15 * dx) / length,
            destination.b[2] - 60,
          ];
          for (const lift of [0, 40]) {
            const departure: Vec3 = [source.a[0], source.a[1], source.a[2] + lift];
            addPath([source.a, ...integratedJumpTrajectory(departure, [target], "up")]);
          }
          // Binding the destination plane keeps the target's map point, raising
          // world Y and Z by sixty. The landing action can lift Z another sixty
          // before its remaining sprite motion settles onto the receiving plane.
          const receiver: Vec3 = [target[0], target[1] + 60, target[2] + 60];
          addPath([target, receiver]);
          // Sprite-specific action timing selects where on this segment the lift
          // happens. Reserve its full height envelope without reading sprites.
          addPath([receiver, destination.b], 60);
        } else {
          // Downward takeoff plays in place; its action point changes only Z.
          const departure: Vec3 = [source.a[0], source.a[1], source.a[2] - 50];
          addPath([source.a, ...integratedJumpTrajectory(departure, [destination.b], "down")]);
        }
      }
      const ribbons: VerticalFlightRibbon[] = [
        ...slopedRibbons,
        ...flights.map(([a, b, extraHeight, padding]): VerticalFlightRibbon => ({
          points: [a, b, [b[0] + dx, b[1] + dy, b[2]], [a[0] + dx, a[1] + dy, a[2]]],
          extraHeight,
          padding,
        })),
      ];
      for (const { points, extraHeight, shift, padding = 0 } of ribbons) {
        const ribbon: Vertex[] = points.map((point, i) => [...point, i < 2 ? 0 : 1]);
        const minX =
          Math.min(...ribbon.map((p) => p[0])) -
          body.radius -
          padding +
          Math.min(0, shift?.[0] ?? 0);
        const maxX =
          Math.max(...ribbon.map((p) => p[0])) +
          body.radius +
          padding +
          Math.max(0, shift?.[0] ?? 0);
        const minY =
          Math.min(...ribbon.map((p) => p[1])) -
          body.radius -
          padding +
          Math.min(0, shift?.[1] ?? 0);
        const maxY =
          Math.max(...ribbon.map((p) => p[1])) +
          body.radius +
          padding +
          Math.max(0, shift?.[1] ?? 0);
        for (const prism of prisms) {
          if (
            maxX < prism.bounds[0] ||
            maxY < prism.bounds[1] ||
            minX > prism.bounds[2] ||
            minY > prism.bounds[3]
          )
            continue;
          if (sloped) {
            const envelope = {
              radius: body.radius + padding,
              height: body.height + extraHeight + 2 * padding,
            };
            const range = ribbonParameterRange(
              points,
              prism.planes.map((plane) => (point) => {
                const value = plane([point[0], point[1], point[2] - padding, 0], envelope);
                return shift
                  ? Math.max(
                      value,
                      plane(
                        [
                          point[0] + shift[0],
                          point[1] + shift[1],
                          point[2] + shift[2] - padding,
                          0,
                        ],
                        envelope,
                      ),
                    )
                  : value;
              }),
            );
            if (range) blocked.push(reverse ? [1 - range[1], 1 - range[0]] : range);
            continue;
          }
          let intersection = ribbon;
          for (const plane of prism.planes) {
            const envelope = { ...body, height: body.height + extraHeight };
            intersection = clip(intersection, (point) => {
              const value = plane(point, envelope);
              if (!shift) return value;
              const translated: Vertex = [
                point[0] + shift[0],
                point[1] + shift[1],
                point[2] + shift[2],
                point[3],
              ];
              return Math.max(value, plane(translated, envelope));
            });
            if (!intersection.length) break;
          }
          if (intersection.length) {
            const lo = Math.min(...intersection.map((v) => v[3]));
            const hi = Math.max(...intersection.map((v) => v[3]));
            blocked.push(reverse ? [1 - hi, 1 - lo] : [lo, hi]);
          }
        }
      }
    }
    return mergeIntervals(blocked);
  };
}

export function trimJumpEdges(
  edges: [JumpEdge, JumpEdge],
  low: number,
  high: number,
): [JumpEdge, JumpEdge] {
  const interpolate = (edge: JumpEdge, t: number): Vec3 =>
    edge.a.map((n, i) => n + t * (edge.b[i]! - n)) as Vec3;
  return [
    { ...edges[0], a: interpolate(edges[0], low), b: interpolate(edges[0], high) },
    { ...edges[1], a: interpolate(edges[1], 1 - high), b: interpolate(edges[1], 1 - low) },
  ];
}

/** Preserve equal, opposing integer vectors so runtime distance-along-edge stays aligned. */
export function snapJumpEdges(edges: [JumpEdge, JumpEdge]): [JumpEdge, JumpEdge] {
  const snap = (p: Vec3): Vec3 => {
    const z = Math.ceil(p[2] - EPSILON) + 0;
    return [Math.round(p[0]), Math.round(p[1] - p[2]) + z, z];
  };
  const a = snap(edges[0].a),
    b = snap(edges[0].b),
    oppositeB = snap(edges[1].b);
  const z = Math.ceil(edges[1].a[2] - EPSILON) + 0;
  const oppositeA: Vec3 = [
    oppositeB[0] + b[0] - a[0],
    oppositeB[1] - oppositeB[2] + b[1] - b[2] - a[1] + a[2] + z,
    z,
  ];
  return [
    { ...edges[0], a, b },
    { ...edges[1], a: oppositeA, b: oppositeB },
  ];
}
