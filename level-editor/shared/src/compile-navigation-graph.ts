import type { CompiledAssetGeometry } from "./asset-gameplay.ts";
import type { Point } from "./level.ts";
import { pointInGameplayPolygon } from "./navigation-anchor.ts";
import { NAVIGATION_HALF_DIAGONAL as half } from "./navigation-footprint.ts";

type Layers = CompiledAssetGeometry["motion_data"]["layers"];
type Address = [number, number, number, number];
interface Node {
  address: Address;
  position: Point;
  from: Point;
  to: Point;
  docking: Point;
  place: number;
  state: number;
  links: number[];
}
interface Link {
  from: Node;
  to: Node;
  state: number;
  distance: number;
}

const offsets: Point[] = [
  [-half[0], -half[1]],
  [half[0], -half[1]],
  [half[0], half[1]],
  [-half[0], half[1]],
];
const insetHalf: Point = [half[0] - 1, half[1] - 1];

function cross(a: Point, b: Point, c: Point): number {
  return (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
}

/** Swept movement footprint, with the normal one-unit movement inset. */
function sweep(a: Point, b: Point): Point[] {
  const points: Point[] = [a, b].flatMap(([x, y]) => [
    [x - insetHalf[0], y - insetHalf[1]],
    [x + insetHalf[0], y - insetHalf[1]],
    [x + insetHalf[0], y + insetHalf[1]],
    [x - insetHalf[0], y + insetHalf[1]],
  ]);
  // Horizontal native corridors shift their left edge one unit farther out.
  if (a[1] === b[1] && a[0] !== b[0]) {
    points.push([Math.min(a[0], b[0]) - half[0], a[1] - insetHalf[1]]);
    points.push([Math.min(a[0], b[0]) - half[0], a[1] + insetHalf[1]]);
  }
  points.sort((p, q) => p[0] - q[0] || p[1] - q[1]);
  const side = (input: Point[]) => {
    const result: Point[] = [];
    for (const point of input) {
      while (result.length > 1 && cross(result.at(-2)!, result.at(-1)!, point) <= 0) result.pop();
      result.push(point);
    }
    result.pop();
    return result;
  };
  return [...side(points), ...side([...points].reverse())];
}

function bounds(points: Point[]): [number, number, number, number] {
  return [
    Math.min(...points.map((p) => p[0])),
    Math.min(...points.map((p) => p[1])),
    Math.max(...points.map((p) => p[0])),
    Math.max(...points.map((p) => p[1])),
  ];
}

function disjoint(a: number[], b: number[]): boolean {
  return a[2]! < b[0]! || b[2]! < a[0]! || a[3]! < b[1]! || b[3]! < a[1]!;
}

function edges(points: Point[]) {
  return points.map((a, index) => {
    const b = points[(index + 1) % points.length]!;
    return {
      a,
      b,
      box: [Math.min(a[0], b[0]), Math.min(a[1], b[1]), Math.max(a[0], b[0]), Math.max(a[1], b[1])],
    };
  });
}

function touches(a: ReturnType<typeof edges>, b: ReturnType<typeof edges>): boolean {
  for (const { a: p, b: q, box } of a) {
    for (const { a: r, b: s, box: other } of b) {
      if (disjoint(box, other)) continue;
      if (cross(p, q, r) * cross(p, q, s) <= 0 && cross(r, s, p) * cross(r, s, q) <= 0) return true;
    }
  }
  return false;
}

/** Required-state words use one of two bits for each independently switched pair. */
function consistent(mask: number): boolean {
  for (let bit = 0; bit < 32; bit += 2) if (((mask >>> bit) & 3) === 3) return false;
  return true;
}

function avoid(masks: number[], blocked: number): number[] {
  if (blocked === 0) return [];
  const alternatives: number[] = [];
  for (let bit = 0; bit < 32; bit++)
    if ((blocked & (1 << bit)) !== 0) alternatives.push((1 << (bit ^ 1)) >>> 0);
  const result = [
    ...new Set(masks.flatMap((mask) => alternatives.map((other) => (mask | other) >>> 0))),
  ].filter(consistent);
  return result.filter(
    (mask) => !result.some((other) => other !== mask && (mask & other) >>> 0 === other),
  );
}

/**
 * Prepare export-time corridor checks. Each returned mask is an alternative
 * required state; no alternatives means blocked, and [0] means unconditional.
 * Pass a floor for walking links. Animated passages cross a floor boundary and
 * therefore check obstacles without requiring the footprint inside that floor.
 */
export function prepareCorridorStates(
  obstacles: Layers[number][number]["obstacles"],
  floor?: Point[],
): (a: Point, b: Point) => number[] {
  const floorEdges = floor && edges(floor);
  const blockers = obstacles.map((obstacle) => ({
    points: obstacle.polygon.points,
    edges: edges(obstacle.polygon.points),
    bounds: bounds(obstacle.polygon.points),
    state: obstacle.state_id >>> 0,
  }));
  return (a, b) => {
    const footprint = sweep(a, b);
    const footprintEdges = edges(footprint);
    if (
      floor &&
      (!pointInGameplayPolygon(footprint[0]!, floor) || touches(footprintEdges, floorEdges!))
    )
      return [];
    const box = bounds(footprint);
    let states = [0];
    for (const obstacle of blockers) {
      if (
        disjoint(box, obstacle.bounds) ||
        (!touches(footprintEdges, obstacle.edges) &&
          !pointInGameplayPolygon(footprint[0]!, obstacle.points) &&
          !pointInGameplayPolygon(obstacle.points[0]!, footprint))
      )
        continue;
      states = avoid(states, obstacle.state);
      if (!states.length) break;
    }
    return states;
  };
}

/**
 * Construct the native graph stream from placed motion geometry at export time.
 * Each node owns one docking position, so switching between sides of a corner
 * requires an explicitly clearance-checked link, including its state constraints.
 */
export function compileNavigationGraph(layers: Layers): number[] {
  const links: Link[] = [];
  const hierarchy = layers.map((areas, layer) =>
    areas.map((area, areaIndex) => {
      // These are the contours consumed by the native movement grid.
      const allowed = prepareCorridorStates(area.obstacles, area.polygon.points);
      const nodes: Node[] = [];
      for (const [ringIndex, raw] of [
        area.polygon.points,
        ...area.obstacles.map((obstacle) => obstacle.polygon.points),
      ].entries()) {
        const ring =
          raw.length > 1 && raw[0]![0] === raw.at(-1)![0] && raw[0]![1] === raw.at(-1)![1]
            ? raw.slice(0, -1)
            : raw;
        const winding = ring.reduce((sum, p, index) => {
          const q = ring[(index + 1) % ring.length]!;
          return sum + p[0] * q[1] - q[0] * p[1];
        }, 0);
        const sign = Math.sign(winding) * (ringIndex === 0 ? 1 : -1);
        for (const [index, position] of ring.entries()) {
          const before = ring[(index + ring.length - 1) % ring.length]!;
          const after = ring[(index + 1) % ring.length]!;
          // Routes bend around obstacle corners and inward floor corners.
          // Outward floor corners and collinear vertices do not create detours.
          if (cross(before, position, after) * sign >= 0) continue;
          const to: Point = [(position[0] - before[0]) * sign, (position[1] - before[1]) * sign];
          const from: Point = [(after[0] - position[0]) * sign, (after[1] - position[1]) * sign];
          for (const [place, offset] of offsets.entries()) {
            const docking: Point = [position[0] + offset[0], position[1] + offset[1]];
            for (const state of allowed(docking, docking)) {
              nodes.push({
                address: [layer, areaIndex, 0, nodes.length],
                position,
                from,
                to,
                docking,
                place,
                state,
                links: [],
              });
            }
          }
        }
      }
      for (let a = 0; a < nodes.length; a++) {
        const from = nodes[a]!;
        for (let b = a + 1; b < nodes.length; b++) {
          const to = nodes[b]!;
          if (from.docking[0] === to.docking[0] && from.docking[1] === to.docking[1]) continue;
          const distance = Math.hypot(
            from.docking[0] - to.docking[0],
            from.docking[1] - to.docking[1],
          );
          for (const state of allowed(from.docking, to.docking)) {
            for (const [start, end] of [
              [from, to],
              [to, from],
            ]) {
              start!.links.push(links.length);
              links.push({ from: start!, to: end!, distance, state });
            }
          }
        }
      }
      return nodes;
    }),
  );
  const bytes: number[] = [];
  const scalar = new DataView(new ArrayBuffer(4));
  const u8 = (value: number) => {
    bytes.push(value);
  };
  const u16 = (value: number) => {
    if (!Number.isInteger(value) || value < 0 || value > 65535)
      throw new Error(`Navigation graph unsigned value out of range: ${value}`);
    bytes.push(value & 255, value >>> 8);
  };
  const i16 = (value: number) => {
    if (!Number.isInteger(value) || value < -32768 || value > 32767)
      throw new Error(`Navigation graph coordinate out of range: ${value}`);
    u16(value & 65535);
  };
  const u32 = (value: number) => {
    if (!Number.isInteger(value) || value < 0 || value > 0xffffffff)
      throw new Error(`Navigation graph link value out of range: ${value}`);
    u16(value & 65535);
    u16(value >>> 16);
  };
  const f32 = (value: number) => {
    scalar.setFloat32(0, value, true);
    for (let index = 0; index < 4; index++) u8(scalar.getUint8(index));
  };
  // The extended stream changes only link counts and indices to 32 bits.
  // Node addresses, geometry and runtime search semantics remain identical.
  const wideLinks = links.length > 65535;
  const linkIndex = wideLinks ? u32 : u16;
  if (wideLinks) {
    u16(65535);
    u16(1);
  }
  u16(1);
  f32(half[0]);
  f32(half[1]);
  u16(hierarchy.length);
  for (const layer of hierarchy) {
    u16(layer.length);
    for (const nodes of layer) {
      u16(1);
      u16(nodes.length);
      for (const node of nodes) {
        u16(1);
        u8(1 << node.place);
        node.position.forEach(i16);
        node.from.forEach(i16);
        node.to.forEach(i16);
        u32(node.state);
        linkIndex(node.links.length);
        node.links.forEach(linkIndex);
      }
    }
  }
  linkIndex(links.length);
  for (const link of links) {
    link.to.address.forEach(u16);
    link.from.address.forEach(u16);
    f32(link.distance);
    u32(link.state);
    u16(1);
    u16(link.from.place * 4 + link.to.place);
  }
  u16(16);
  for (let from = 0; from < 4; from++)
    for (let to = 0; to < 4; to++) {
      u8(1 << from);
      u8(1 << to);
      u16(1);
      u8(1 << from);
      u16(1);
      u8(1 << to);
    }
  return bytes;
}
