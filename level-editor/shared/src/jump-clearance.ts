import earcut from "earcut";
import type { SightObstacle } from "./level.ts";
import type { Vec3 } from "./scene.ts";
import { heightPlane, planeHeight } from "./gameplay-plane.ts";

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

export function mergeIntervals(intervals: Interval[]): Interval[] {
  const result: Interval[] = [];
  for (const [a, b] of [...intervals].sort((a, b) => a[0] - b[0])) {
    const last = result.at(-1);
    if (last && a <= last[1] + EPSILON) last[1] = Math.max(last[1], b);
    else result.push([a, b]);
  }
  return result;
}

/** Precompute solid prisms once, then intersect the full flight ribbon, not sampled rays. */
export function createJumpClearance(obstacles: SightObstacle[]) {
  const prisms = obstacles
    .filter((shape) => shape.solid)
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
    if (!long && Math.abs(edges[0].a[2] - edges[1].a[2]) >= 60)
      throw new Error(
        "Climbing jumps need an authored connection; automatic flight clearance supports long jumps",
      );
    const blocked: Interval[] = [];
    for (const [source, destination, reverse] of [
      [edges[0], edges[1], false],
      [edges[1], edges[0], true],
    ] as const) {
      const dx = source.b[0] - source.a[0],
        dy = source.b[1] - source.a[1];
      const length = Math.hypot(dx, dy);
      if (length < EPSILON) throw new Error("Jump edge collapses on the movement grid");
      const start: Vec3 = [
        source.a[0] - (15 * dy) / length,
        source.a[1] + (15 * dx) / length,
        source.a[2],
      ];
      const path = [source.a, ...longJumpTrajectory(start, destination.b)];
      for (let i = 0; i + 1 < path.length; i++) {
        const a = path[i]!,
          b = path[i + 1]!;
        const ribbon: Vertex[] = [
          [...a, 0],
          [...b, 0],
          [b[0] + dx, b[1] + dy, b[2], 1],
          [a[0] + dx, a[1] + dy, a[2], 1],
        ];
        const minX = Math.min(...ribbon.map((p) => p[0])) - body.radius;
        const maxX = Math.max(...ribbon.map((p) => p[0])) + body.radius;
        const minY = Math.min(...ribbon.map((p) => p[1])) - body.radius;
        const maxY = Math.max(...ribbon.map((p) => p[1])) + body.radius;
        for (const prism of prisms) {
          if (
            maxX < prism.bounds[0] ||
            maxY < prism.bounds[1] ||
            minX > prism.bounds[2] ||
            minY > prism.bounds[3]
          )
            continue;
          let intersection = ribbon;
          for (const plane of prism.planes) {
            intersection = clip(intersection, (point) => plane(point, body));
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
    const z = Math.round(p[2]);
    return [Math.round(p[0]), Math.round(p[1] - p[2]) + z, z];
  };
  const a = snap(edges[0].a),
    b = snap(edges[0].b),
    oppositeB = snap(edges[1].b);
  const z = Math.round(edges[1].a[2]);
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
