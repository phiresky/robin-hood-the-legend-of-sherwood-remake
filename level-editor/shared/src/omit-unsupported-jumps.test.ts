import test from "node:test";
import assert from "node:assert/strict";
import type { JumpLinePair, JumpZone, Point, SightObstacle } from "./level.ts";
import { omitUnsupportedJumps } from "./omit-unsupported-jumps.ts";

const zone = (sector: number): JumpZone => ({
  polygon: { points: [] },
  sector,
  layer: 1,
  helper_needed: false,
});
const receiver = (sector: number, polygon: Point[], height: number): SightObstacle =>
  ({
    points: polygon.map(([x, y]) => ({ x, y: y + height, z_top: height })),
    projection_area: [sector, 1],
  }) as unknown as SightObstacle;
const square = (x: number, y: number): Point[] => [
  [x, y],
  [x + 20, y],
  [x + 20, y + 20],
  [x, y + 20],
];
// Line 1 stands on zone 1's surface and lands in zone 0; line 2 the reverse.
const pair = (lineTwoX: number, height = 10): JumpLinePair => ({
  jump_long: false,
  line1: { point_a: [5, 5, height], point_b: [15, 5, height], jump_zone_index: 0 },
  line2: {
    point_a: [lineTwoX + 15, 5, height],
    point_b: [lineTwoX + 5, 5, height],
    jump_zone_index: 1,
  },
});

test("jumps with an elevated line entirely off its receiving surface are omitted", () => {
  const sight = [receiver(10, square(0, 0), 10), receiver(20, square(100, 0), 10)];
  const zones = [zone(20), zone(10), zone(30)];
  const kept = { pair: pair(100), sourceIndex: 0 };
  const result = omitUnsupportedJumps([kept], zones, sight, (i) => `pair-${i}`);
  assert.deepEqual(result.pairs, [kept]);
  assert.equal(result.zones, zones);
  assert.deepEqual(result.warnings, []);

  // The second line now lies beyond every face of its own surface.
  const lost = omitUnsupportedJumps(
    [
      { pair: pair(300), sourceIndex: 0 },
      { pair: { ...pair(100), line1: { ...pair(100).line1, jump_zone_index: 2 } }, sourceIndex: 1 },
    ],
    zones,
    [...sight, receiver(30, square(100, 0), 10)],
    (i) => `pair-${i}`,
  );
  assert.equal(lost.pairs.length, 1);
  assert.equal(lost.pairs[0]!.sourceIndex, 1);
  assert.deepEqual(lost.zones, [zones[1], zones[2]]);
  assert.equal(lost.pairs[0]!.pair.line1.jump_zone_index, 1);
  assert.equal(lost.pairs[0]!.pair.line2.jump_zone_index, 0);
  assert.match(
    lost.warnings[0]!,
    /^Jump pair-0: line 2 has no receiving surface able to hold an actor/,
  );

  // A sliver touched by the line still cannot hold the 12×6 actor footprint.
  const sliver = omitUnsupportedJumps(
    [{ pair: pair(100), sourceIndex: 0 }],
    zones,
    [
      receiver(10, square(0, 0), 10),
      receiver(
        20,
        [
          [100, 4],
          [120, 4],
          [120, 6],
          [100, 6],
        ],
        10,
      ),
    ],
    String,
  );
  assert.equal(sliver.pairs.length, 0);

  // A partially covered line is left to the coverage audits.
  const partial = omitUnsupportedJumps([{ pair: pair(110), sourceIndex: 0 }], zones, sight, String);
  assert.equal(partial.pairs.length, 1);
});

test("ground-level jump lines may use the implicit ground plane", () => {
  const sight = [receiver(10, square(0, 0), 10)];
  const ground = pair(300, 0);
  ground.line1.point_a[2] = 10;
  ground.line1.point_b[2] = 10;
  const result = omitUnsupportedJumps(
    [{ pair: ground, sourceIndex: 0 }],
    [zone(20), zone(10)],
    sight,
    String,
  );
  assert.equal(result.pairs.length, 1);
  // Surfaces without receiving faces are not judged.
  assert.equal(
    omitUnsupportedJumps([{ pair: pair(300), sourceIndex: 0 }], [zone(40), zone(10)], sight, String)
      .pairs.length,
    1,
  );
});
