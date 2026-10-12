import test from "node:test";
import assert from "node:assert/strict";
import { partitionJumpReceivers } from "./partition-jump-receivers.ts";
import type { JumpLinePair, JumpZone, Point, SightObstacle } from "./level.ts";
import type { Vec3 } from "./scene.ts";

function fixture() {
  const zones: JumpZone[] = [0, 1].map((sector) => ({
    sector,
    layer: 1,
    helper_needed: false,
    polygon: {
      points: [
        [0, sector * 100 - 20],
        [100, sector * 100 - 20],
        [100, sector * 100 + 20],
        [0, sector * 100 + 20],
      ],
    },
  }));
  const pairs: JumpLinePair[] = [
    {
      line1: { point_a: [10, 0, 20], point_b: [90, 0, 20], jump_zone_index: 1 },
      line2: { point_a: [90, 100, 20], point_b: [10, 100, 20], jump_zone_index: 0 },
      jump_long: true,
    },
  ];
  const face = (sector: number, low: number, high: number, slope: number): SightObstacle => ({
    projection_area: [sector, 1],
    points: [
      [low, sector * 100 - 20],
      [high, sector * 100 - 20],
      [high, sector * 100 + 20],
      [low, sector * 100 + 20],
    ].map(([x, y]) => {
      const z = 20 + slope * (x! - 50);
      return { x: x!, y: y! + z, z_bottom: z, z_top: z };
    }),
    opaque: false,
    solid: false,
    mouse: true,
    show_shadow_polygon: false,
    default_material: 0,
    material_indices: [],
  });
  return {
    zones,
    pairs,
    face,
    sight: [face(0, 0, 100, 0), face(1, 0, 50, 0), face(1, 50, 100, 0.1)],
  };
}

test("partitioned roof landings have paired spans and face-specific click zones", () => {
  const { pairs, zones, sight } = fixture();
  const before = structuredClone({ pairs, zones, sight });
  const result = partitionJumpReceivers(pairs, zones, sight);
  assert.equal(result.pairs.length, 2);
  assert.deepEqual(result.warnings, []);
  assert.deepEqual(
    result.pairs.map(({ pair }) => [pair.line1.point_a[0], pair.line1.point_b[0]]),
    [
      [10, 50],
      [50, 90],
    ],
  );
  for (const { pair } of result.pairs) {
    assert.equal(
      pair.line1.point_b[0] - pair.line1.point_a[0],
      pair.line2.point_a[0] - pair.line2.point_b[0],
    );
    const destination = result.zones[pair.line1.jump_zone_index]!;
    const xs = destination.polygon.points.map((p) => p[0]);
    if (pair.line1.point_a[0] < 50) assert.equal(Math.max(...xs), 50);
    else assert.equal(Math.min(...xs), 50);
  }
  assert.deepEqual({ pairs, zones, sight }, before);
});

test("coplanar receiving pieces do not subdivide native jumps", () => {
  const { pairs, zones, face } = fixture();
  const result = partitionJumpReceivers(pairs, zones, [
    face(0, 0, 100, 0),
    face(1, 0, 50, 0),
    face(1, 50, 100, 0),
  ]);
  assert.deepEqual(
    result.pairs.map((p) => p.pair),
    pairs,
  );
  assert.deepEqual(result.zones, zones);
  assert.deepEqual(result.warnings, []);
});

test("uncovered roof gaps remain explicit warnings rather than invented support", () => {
  const { pairs, zones, face } = fixture();
  const result = partitionJumpReceivers(pairs, zones, [
    face(0, 0, 100, 0),
    face(1, 0, 49, 0),
    face(1, 51, 100, 0.1),
  ]);
  assert.deepEqual(
    result.pairs.map((p) => p.pair),
    pairs,
  );
  assert.deepEqual(result.zones, zones);
  assert.match(result.warnings[0]!, /uncovered gaps/);
});

test("receiver partitions follow rotated translated and elevated geometry", () => {
  for (const degrees of [0, 37, 90, 180]) {
    for (const elevation of [0, 40]) {
      const { pairs, zones, sight } = fixture();
      zones[1]!.helper_needed = true;
      const radians = (degrees * Math.PI) / 180;
      const transform = ([x, y]: Point): Point => [
        400 + Math.cos(radians) * x - Math.sin(radians) * y,
        400 + Math.sin(radians) * x + Math.cos(radians) * y - elevation,
      ];
      const transformLine = ([x, y, z]: Vec3): Vec3 => {
        const point = transform([x, y]);
        return [Math.round(point[0]), Math.round(point[1]), z + elevation];
      };
      for (const zone of zones) zone.polygon.points = zone.polygon.points.map(transform);
      for (const line of [pairs[0]!.line1, pairs[0]!.line2]) {
        line.point_a = transformLine(line.point_a);
        line.point_b = transformLine(line.point_b);
      }
      for (const receiver of sight) {
        receiver.points = receiver.points.map((p) => {
          const [x, y] = transform([p.x, p.y - p.z_top]);
          const z = p.z_top + elevation;
          return { x, y: y + z, z_top: z, z_bottom: z };
        });
      }
      const result = partitionJumpReceivers(pairs, zones, sight);
      assert.equal(result.pairs.length, 2, `${degrees}/${elevation}`);
      assert.deepEqual(result.warnings, []);
      for (const { pair } of result.pairs) {
        assert.equal(result.zones[pair.line1.jump_zone_index]!.helper_needed, true);
        assert.equal(result.zones[pair.line2.jump_zone_index]!.helper_needed, false);
      }
    }
  }
});

test("a subpixel receiver partition warns and retains the complete authored connection", () => {
  const { pairs, zones, face } = fixture();
  const result = partitionJumpReceivers(pairs, zones, [
    face(0, 0, 100, 0),
    face(1, 0, 10.1, 0),
    face(1, 10.1, 100, 0.1),
  ]);
  assert.deepEqual(
    result.pairs.map((p) => p.pair),
    pairs,
  );
  assert.deepEqual(result.zones, zones);
  assert.match(result.warnings[0]!, /span collapses/);
});
