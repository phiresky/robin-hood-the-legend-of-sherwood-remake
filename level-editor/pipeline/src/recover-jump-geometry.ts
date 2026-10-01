import earcut, { flatten } from "earcut";
import clipping from "polygon-clipping";
import type { JumpZone, Point, ProtoLevel } from "../../shared/src/level.ts";
import type { Vec3 } from "../../shared/src/scene.ts";
import { distanceToPolygon } from "./recovery-elevation.ts";
import type {
  AssetJumpPair,
  AssetJumpZone,
  AssetJumpSegment,
} from "../../shared/src/asset-gameplay.ts";

export function recoverJumpSegment(
  side: 0 | 1,
  ...args: Parameters<typeof recoverJumpGeometry>
): { zone: AssetJumpZone; segment: AssetJumpSegment & { join: Vec3 } } {
  const [proto, pairIndex, node, localize, heightAt, receivingFootprints] = args;
  const sourcePair = proto.jump_line_pairs[pairIndex];
  if (!sourcePair) throw new Error(`Missing jump pair ${pairIndex}`);
  const home = proto.jump_zones[(side === 0 ? sourcePair.line2 : sourcePair.line1).jump_zone_index];
  const recovered = recoverJumpGeometry(
    proto,
    pairIndex,
    node,
    localize,
    heightAt,
    receivingFootprints
      ? (zone) => (zone === home ? receivingFootprints(zone) : undefined)
      : undefined,
  );
  const edge = recovered.pair.edges[side];
  const zone = recovered.zones.find((zone) => zone.id === edge.zone)!;
  const endpoints = recovered.pair.edges.flatMap((edge) => [edge.a, edge.b]);
  const join = [0, 1, 2].map((axis) => endpoints.reduce((sum, p) => sum + p[axis]!, 0) / 4) as Vec3;
  return {
    zone,
    segment: {
      id: `${recovered.pair.id}-side-${side}`,
      node: recovered.pair.node,
      long: recovered.pair.long,
      join,
      edge,
    },
  };
}

/** Recover a complete pair into an explicit owner; source indices never enter gameplay links. */
export function recoverJumpGeometry(
  proto: Pick<ProtoLevel, "motion_data" | "jump_zones" | "jump_line_pairs">,
  pairIndex: number,
  node: string,
  localize: (p: Vec3) => Vec3,
  heightAt: (zone: JumpZone, point: Point) => number,
  receivingFootprints?: (zone: JumpZone) => Point[][] | undefined,
): { zones: AssetJumpZone[]; pair: AssetJumpPair } {
  const pair = proto.jump_line_pairs[pairIndex];
  if (!pair) throw new Error(`Missing jump pair ${pairIndex}`);
  const lines = [pair.line1, pair.line2];
  const closed = (points: Point[]) => [[...points, points[0]!]];
  const zones = [...new Set(lines.map((line) => line.jump_zone_index))]
    .sort((a, b) => a - b)
    .map((index): AssetJumpZone => {
      const zone = proto.jump_zones[index];
      if (!zone) throw new Error(`Missing jump zone ${index}`);
      let sector = 0;
      const area = proto.motion_data.layers.flatMap((areas, layer) =>
        areas.flatMap((area) => {
          const match = sector === zone.sector && layer === zone.layer;
          sector += 1 + area.obstacles.length;
          return match ? [area] : [];
        }),
      )[0];
      if (!area || area.is_lift)
        throw new Error(`Jump zone ${index} has no ordinary movement area`);
      let free = clipping.intersection(closed(zone.polygon.points), closed(area.polygon.points));
      const blockers = area.obstacles.filter((o) => {
        if (
          typeof o.state_id !== "number" ||
          !Number.isInteger(o.state_id) ||
          o.state_id < 0 ||
          o.state_id > 0xffffffff
        )
          throw new Error("Jump receiving area has invalid obstacle state");
        return (o.state_id & 0x55555555) === o.state_id;
      });
      if (blockers.length)
        free = clipping.difference(free, ...blockers.map((o) => closed(o.polygon.points)));
      const footprints = receivingFootprints?.(zone);
      if (footprints !== undefined) {
        if (!footprints.length)
          throw new Error(`Jump zone ${index} has no owned receiving footprint`);
        free = clipping.intersection(
          free,
          clipping.union(closed(footprints[0]!), ...footprints.slice(1).map(closed)),
        );
      }
      const midpoint: Point = [0, 1].map(
        (axis) =>
          zone.polygon.points.reduce((sum, p) => sum + p[axis]!, 0) / zone.polygon.points.length,
      ) as Point;
      const candidates: Point[] = [];
      for (const region of free) {
        const { vertices, holes, dimensions } = flatten(region);
        const triangles = earcut(vertices, holes, dimensions);
        for (let i = 0; i < triangles.length; i += 3) {
          const indices = triangles.slice(i, i + 3);
          candidates.push([
            indices.reduce((s, j) => s + vertices[j * 2]!, 0) / 3,
            indices.reduce((s, j) => s + vertices[j * 2 + 1]!, 0) / 3,
          ]);
        }
      }
      candidates.sort(
        (a, b) =>
          Math.hypot(a[0] - midpoint[0], a[1] - midpoint[1]) -
          Math.hypot(b[0] - midpoint[0], b[1] - midpoint[1]),
      );
      // Compilation resolves anchors on the integer movement grid. Evaluate
      // elevation at that same point, while retaining only unblocked owner coverage.
      const anchor = candidates
        .map(([x, y]): Point => [Math.round(x), Math.round(y)])
        .find((point) =>
          free.some(
            (region) =>
              distanceToPolygon(point, region[0] as Point[]) === 0 &&
              region.slice(1).every((hole) => distanceToPolygon(point, hole as Point[]) > 0),
          ),
        );
      if (!anchor) throw new Error(`Jump zone ${index} has no unblocked landing anchor`);
      const z = heightAt(zone, anchor);
      if (!Number.isFinite(z))
        throw new Error(`Jump zone ${index} has invalid receiving elevation`);
      return {
        id: `jump-zone-${index}`,
        node,
        anchor: localize([anchor[0], anchor[1] + z, z]),
        polygon: zone.polygon.points.map(([x, y]) => localize([x, y + z, z])),
        helperNeeded: zone.helper_needed,
      };
    });
  const point = (p: Vec3): Vec3 => {
    if (p.length !== 3 || !p.every(Number.isFinite))
      throw new Error("Jump edge lacks full 3D geometry");
    return localize([p[0], p[1] + p[2], p[2]]);
  };
  return {
    zones,
    pair: {
      id: `jump-pair-${pairIndex}`,
      node,
      long: pair.jump_long,
      edges: [
        {
          zone: `jump-zone-${pair.line2.jump_zone_index}`,
          a: point(pair.line1.point_a),
          b: point(pair.line1.point_b),
        },
        {
          zone: `jump-zone-${pair.line1.jump_zone_index}`,
          a: point(pair.line2.point_a),
          b: point(pair.line2.point_b),
        },
      ],
    },
  };
}
