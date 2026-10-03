import test from "node:test";
import assert from "node:assert/strict";
import { generateJumpLedges, jumpLandingBand } from "./generate-jump-ledges.ts";
import type { Point } from "./level.ts";
import { readFile } from "node:fs/promises";
import type { SightObstacle } from "./level.ts";
import type { AssetWalkableSurface } from "./asset-gameplay.ts";
import { heightPlane } from "./gameplay-plane.ts";
import { createJumpClearance } from "./jump-clearance.ts";
import { assembleJumpSegments } from "./assemble-jump-segments.ts";
import type { Vec3 } from "./scene.ts";

const polygon: Point[] = [
  [0, 0],
  [100, 0],
  [100, 100],
  [0, 100],
];
const rules = { maxGap: 100, maxRise: 40, maxDrop: 40, minOverlap: 10, inset: 5, landingDepth: 10 };

test("surface ledges face outward, with receiving bands entirely inside the surface", () => {
  const generated = generateJumpLedges("roof", polygon, [], [0, 0, 20], rules);
  assert.equal(generated.segments.length, 4);
  assert.deepEqual(generated.warnings, []);
  for (const segment of generated.segments) {
    const zone = jumpLandingBand(
      segment.id,
      segment.edge,
      generated.landings.get(segment.edge.zone)!,
    );
    assert.equal(zone.anchor[2], 20);
    assert.ok(zone.polygon.every(([x, y]) => x >= 0 && x <= 100 && y >= 0 && y <= 100));
    assert.ok(!segment.join);
  }
  assert.deepEqual(generated.segments[1]!.edge.a, [95, 119, 20]);
  assert.deepEqual(generated.segments[1]!.edge.b, [95, 21, 20]);
});

test("holes cut receiving bands into independent usable spans", () => {
  const hole: Point[] = [
    [80, 40],
    [99, 40],
    [99, 60],
    [80, 60],
  ];
  const generated = generateJumpLedges("roof", polygon, [hole], [0, 0, 0], {
    ...rules,
    edges: [1],
  });
  assert.equal(generated.segments.length, 2);
  const edges = generated.segments.map((segment) => segment.edge);
  assert.deepEqual(
    edges.map((edge) => [edge.a[1], edge.b[1]]),
    [
      [99, 60],
      [40, 1],
    ],
  );
});

test("sloping surfaces expose only level takeoff edges and preserve receiving elevation", () => {
  const generated = generateJumpLedges("roof", polygon, [], [0, 1, 0], rules);
  assert.equal(generated.segments.length, 2);
  assert.equal(generated.warnings.length, 2);
  const segment = generated.segments[0]!;
  const zone = jumpLandingBand(
    segment.id,
    segment.edge,
    generated.landings.get(segment.edge.zone)!,
  );
  assert.equal(zone.anchor[2], 10);
});

test("authored adjustment derives level contours inside a slightly skewed roof boundary", () => {
  const skewed: Point[] = [
    [0, 0],
    [100, 1],
    [100, 101],
    [0, 100],
  ];
  const plane = [0, 0.5, 20] as const;
  const generated = generateJumpLedges("roof", skewed, [], [...plane], {
    ...rules,
    maxLevelAdjustment: 1,
    edges: [0, 2],
  });
  assert.equal(generated.segments.length, 2);
  assert.deepEqual(generated.warnings, []);
  for (const segment of generated.segments) {
    assert.equal(segment.edge.a[2], segment.edge.b[2]);
    for (const [, y, z] of [segment.edge.a, segment.edge.b])
      assert.ok(Math.abs(z - (0.5 * (y - z) + 20)) < 1e-6);
    const band = jumpLandingBand(
      segment.id,
      segment.edge,
      generated.landings.get(segment.edge.zone)!,
    );
    assert.ok(
      band.polygon.every(([x, y]) => x >= 0 && x <= 100 && y >= x / 100 && y <= 100 + x / 100),
    );
  }
  const refused = generateJumpLedges("roof", skewed, [], [...plane], {
    ...rules,
    maxLevelAdjustment: 0.1,
    edges: [0, 2],
  });
  assert.equal(refused.segments.length, 0);
  assert.equal(refused.warnings.length, 2);
});

for (const source of ["rock", "roof"])
  test(`a recovered ${source} surface derives jumps to a rotated copy without saved jump records`, async () => {
    const { surface, obstacle } = JSON.parse(
      await readFile(
        new URL(`../test-fixtures/${source}-jump-source.json`, import.meta.url),
        "utf8",
      ),
    ) as { surface: AssetWalkableSurface; obstacle: SightObstacle };
    const points = surface.polygon.map(([x, y], i): Vec3 => {
      const z = typeof surface.height === "number" ? surface.height : surface.height[i]!;
      return [x, y - z, z];
    });
    const plane = heightPlane(points);
    const bodyRules = {
      ...rules,
      inset: source === "roof" ? 2 : 8,
      landingDepth: source === "roof" ? 4 : 12,
      minOverlap: 16,
      clearance: { radius: source === "roof" ? 0 : 4, height: 60 },
      maxLevelAdjustment: 2,
    };
    const first = generateJumpLedges(
      "rock-a",
      points.map(([x, y]) => [x, y]),
      [],
      plane,
      bodyRules,
    );
    const edge = first.segments.at(source === "roof" ? -1 : 0)!.edge;
    const dx = edge.b[0] - edge.a[0],
      dy = edge.b[1] - edge.a[1],
      length = Math.hypot(dx, dy);
    const tx = edge.a[0] + edge.b[0] - (dy / length) * 50;
    const ty = edge.a[1] + edge.b[1] + (dx / length) * 50;
    const rotated = points.map(([x, y, z]): Vec3 => [-x + tx, -y - 2 * z + ty, z]);
    const second = generateJumpLedges(
      "rock-b",
      rotated.map(([x, y]) => [x, y]),
      [],
      heightPlane(rotated),
      bodyRules,
    );
    const other = {
      ...obstacle,
      points: obstacle.points.map((p) => ({ ...p, x: -p.x + tx, y: -p.y + ty })),
    };
    const result = assembleJumpSegments(
      [...first.segments, ...second.segments],
      createJumpClearance([obstacle, other]),
    );
    assert.ok(result.pairs.length > 0, JSON.stringify(result.warnings));
    for (const pair of result.pairs) {
      assert.equal(pair.edges[0]!.a[2], pair.edges[0]!.b[2]);
      if (source === "rock") assert.equal(pair.edges[0]!.a[2], 51);
    }
  });
