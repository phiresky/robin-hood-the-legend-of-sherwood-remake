import polygonClipping from "polygon-clipping";
import { heightPlane, type HeightPlane } from "./gameplay-plane.ts";
import { pointInGameplayPolygon as inside } from "./navigation-anchor.ts";
import type { JumpLinePair, JumpZone, Point, SightObstacle } from "./level.ts";
import type { Vec3 } from "./scene.ts";

interface Face {
  polygon: Point[];
  plane: HeightPlane;
}

const samePlane = (a: Face, b: Face) =>
  a.plane.every((value, i) => Math.abs(value - b.plane[i]!) < 1e-7);
const cross = (a: Point, b: Point) => a[0] * b[1] - a[1] * b[0];
const subtract = (a: Point, b: Point): Point => [a[0] - b[0], a[1] - b[1]];
const xy = (p: Vec3): Point => [p[0], p[1]];
const at = (a: Point, b: Point, t: number): Point => [
  a[0] + (b[0] - a[0]) * t,
  a[1] + (b[1] - a[1]) * t,
];
const interpolate = (a: Vec3, b: Vec3, t: number): Vec3 => [
  Math.round(a[0] + (b[0] - a[0]) * t),
  Math.round(a[1] + (b[1] - a[1]) * t),
  Math.ceil(a[2] + (b[2] - a[2]) * t),
];

function intervals(a: Point, b: Point, faces: Face[]) {
  const direction = subtract(b, a);
  const cuts = [0, 1];
  for (const face of faces) {
    for (const [i, p] of face.polygon.entries()) {
      const q = face.polygon[(i + 1) % face.polygon.length]!;
      const edge = subtract(q, p);
      const denominator = cross(direction, edge);
      if (Math.abs(denominator) < 1e-9) continue;
      const offset = subtract(p, a);
      const t = cross(offset, edge) / denominator;
      const u = cross(offset, direction) / denominator;
      if (t > 1e-7 && t < 1 - 1e-7 && u >= 0 && u <= 1) cuts.push(t);
    }
  }
  cuts.sort((x, y) => x - y);
  const unique = cuts.filter((v, i) => i === 0 || v - cuts[i - 1]! > 1e-7);
  return unique.slice(1).map((end, i) => {
    const start = unique[i]!;
    const point = at(a, b, (start + end) / 2);
    const matches = faces.filter((face) => inside(point, face.polygon, true));
    if (matches.some((face) => !samePlane(face, matches[0]!)))
      throw new Error("landing span overlaps different receiving planes");
    return { start, end, face: matches[0] };
  });
}

function clippedZone(zone: JumpZone, face: Face, faces: Face[], midpoint: Point): JumpZone {
  const coplanar = faces.filter((candidate) => samePlane(candidate, face));
  const regions = polygonClipping.intersection(
    [zone.polygon.points],
    polygonClipping.union(
      [coplanar[0]!.polygon],
      ...coplanar.slice(1).map((candidate) => [candidate.polygon]),
    ),
  );
  const matches = regions.filter((region) => inside(midpoint, region[0]!, true));
  if (matches.length !== 1 || matches[0]!.length !== 1)
    throw new Error("receiving face cannot form one clickable landing polygon");
  const rounded = matches[0]![0]!
    .slice(0, -1)
    .map(([x, y]): Point => [Math.round(x), Math.round(y)]);
  const points = rounded.filter((point, i) => {
    const previous = rounded[(i + rounded.length - 1) % rounded.length]!;
    return point[0] !== previous[0] || point[1] !== previous[1];
  });
  const area = points.reduce(
    (sum, point, i) => sum + cross(point, points[(i + 1) % points.length]!),
    0,
  );
  if (points.length < 3 || Math.abs(area) < 1 || !inside(midpoint, points, true))
    throw new Error("receiving landing polygon collapses after movement-grid rounding");
  return { ...zone, polygon: { points } };
}

/** Prepare separate native jumps wherever their landing spans cross roof planes. */
export function partitionJumpReceivers(
  pairs: JumpLinePair[],
  zones: JumpZone[],
  sight: SightObstacle[],
) {
  const byTopology = new Map<string, Face[]>();
  const usedTopologies = new Set(zones.map((zone) => `${zone.sector}/${zone.layer}`));
  for (const obstacle of sight) {
    if (!Array.isArray(obstacle.projection_area)) continue;
    const [sector, layer] = obstacle.projection_area;
    if (!usedTopologies.has(`${sector}/${layer}`)) continue;
    const polygon = obstacle.points.map((p): Point => [p.x, p.y - p.z_top]);
    const vertices = obstacle.projection_plane
      ? obstacle.projection_plane.map(([x, y, z]): Vec3 => [x, y - z, z])
      : obstacle.points.slice(0, 3).map((p): Vec3 => [p.x, p.y - p.z_top, p.z_top]);
    const key = `${sector}/${layer}`;
    const faces = byTopology.get(key) ?? [];
    faces.push({ polygon, plane: heightPlane(vertices) });
    byTopology.set(key, faces);
  }
  const result: { pair: JumpLinePair; sourceIndex: number }[] = [];
  const outputZones = [...zones];
  const warnings: string[] = [];
  for (const [sourceIndex, pair] of pairs.entries()) {
    const checkpoint = outputZones.length;
    try {
      const lines = [pair.line1, pair.line2];
      // Parameter t follows line1; line2 runs in the opposite direction.
      const sides = lines.map((line, side) => {
        const zone = zones[lines[1 - side]!.jump_zone_index]!;
        const faces = byTopology.get(`${zone.sector}/${zone.layer}`) ?? [];
        const a = xy(side === 0 ? line.point_a : line.point_b);
        const b = xy(side === 0 ? line.point_b : line.point_a);
        const spans = intervals(a, b, faces);
        const first = spans.find((span) => span.face)?.face;
        const changes = first && spans.some((span) => span.face && !samePlane(first, span.face));
        if (changes && spans.some((span) => !span.face))
          throw new Error("landing span has uncovered gaps between receiving faces");
        return { zone, faces, spans, changes };
      });
      const cuts = [0, 1];
      for (const side of sides) {
        if (!side.changes) continue;
        for (let i = 1; i < side.spans.length; i++) {
          if (!samePlane(side.spans[i - 1]!.face!, side.spans[i]!.face!))
            cuts.push(side.spans[i]!.start);
        }
      }
      cuts.sort((a, b) => a - b);
      const unique = cuts.filter((v, i) => i === 0 || v - cuts[i - 1]! > 1e-7);
      if (unique.length === 2) {
        result.push({ pair, sourceIndex });
        continue;
      }
      const partitions = unique.slice(1).map((high, i) => {
        const low = unique[i]!;
        const part: JumpLinePair = {
          jump_long: pair.jump_long,
          line1: {
            ...pair.line1,
            point_a: interpolate(pair.line1.point_a, pair.line1.point_b, low),
            point_b: interpolate(pair.line1.point_a, pair.line1.point_b, high),
          },
          line2: {
            ...pair.line2,
            point_a: interpolate(pair.line2.point_b, pair.line2.point_a, high),
            point_b: interpolate(pair.line2.point_b, pair.line2.point_a, low),
          },
        };
        const partLines = [part.line1, part.line2];
        for (const [side, line] of partLines.entries()) {
          if (line.point_a[0] === line.point_b[0] && line.point_a[1] === line.point_b[1])
            throw new Error("receiving span collapses after movement-grid rounding");
          const prepared = sides[side]!;
          if (!prepared.changes) continue;
          const midpoint = at(xy(line.point_a), xy(line.point_b), 0.5);
          const face = prepared.faces.find((candidate) =>
            inside(midpoint, candidate.polygon, true),
          );
          if (!face) throw new Error("rounded landing midpoint has no receiver");
          partLines[1 - side]!.jump_zone_index = outputZones.length;
          outputZones.push(clippedZone(prepared.zone, face, prepared.faces, midpoint));
        }
        return { pair: part, sourceIndex };
      });
      result.push(...partitions);
    } catch (error) {
      if (!(error instanceof Error)) throw error;
      outputZones.length = checkpoint;
      warnings.push(`Jump pair ${sourceIndex}: receiver partition unavailable; ${error.message}`);
      result.push({ pair, sourceIndex });
    }
  }
  const used = new Set(
    result.flatMap(({ pair }) => [pair.line1.jump_zone_index, pair.line2.jump_zone_index]),
  );
  const indices = [...used].sort((a, b) => a - b);
  return {
    pairs: result.map(({ pair, sourceIndex }) => ({
      sourceIndex,
      pair: {
        ...pair,
        line1: { ...pair.line1, jump_zone_index: indices.indexOf(pair.line1.jump_zone_index) },
        line2: { ...pair.line2, jump_zone_index: indices.indexOf(pair.line2.jump_zone_index) },
      },
    })),
    zones: indices.map((i) => outputZones[i]!),
    warnings,
  };
}
