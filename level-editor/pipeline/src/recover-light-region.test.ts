import test from "node:test";
import assert from "node:assert/strict";
import {
  containsLightPolygon,
  recoverLightPlane,
  recoverLightRegion,
  recoverLightRegions,
  recoverLightField,
} from "./recover-light-region.ts";
import { fixedClipping } from "../../shared/src/fixed-polygon-boolean.ts";
import { assetCompilerFixture } from "../../shared/test-fixtures/asset-gameplay.ts";
import type { LightSector, MotionArea, SightObstacle } from "../../shared/src/level.ts";
import { heightPlane } from "../../shared/src/gameplay-plane.ts";
import { lightReceiverIntersection } from "../../shared/src/light-receiver-segment.ts";
import { compileAssetGameplay } from "../../shared/src/compile-asset-gameplay.ts";

test("sloped light attachment stops before neighboring floors in either direction", () => {
  const { hut, document, assets } = assetCompilerFixture();
  const points: [number, number][] = [
    [0, 0],
    [100, 0],
    [100, 100],
    [0, 100],
  ];
  const supports: SightObstacle[] = [0, 1, 2].map((sector) => ({
    ...hut.parts[0]!.obstacle_local_game!,
    projection_area: [sector, sector === 0 ? 1 : 2],
    points: points.map(([x, y]) => {
      const z = sector === 0 ? 10 + x : sector === 1 ? 20 : 100;
      return { x, y: y + z, z_bottom: 0, z_top: z };
    }),
  }));
  const area: MotionArea = {
    is_lift: true,
    state_id: 0,
    flags: 0,
    skeleton_segments: [],
    obstacles: [],
    polygon: { points },
  };
  const { region } = recoverLightField(
    { layer: 1, ambience: 4, polygon: { points } },
    "stair",
    supports,
    [area],
    [0],
  );
  assert.equal(region.receiverSegments!.length, 1);
  const segment = region.receiverSegments![0]!;
  assert.ok(lightReceiverIntersection(segment, [1, 0, 10]));
  assert.equal(lightReceiverIntersection(segment, [0, 0, 20]), undefined);
  assert.equal(lightReceiverIntersection(segment, [0, 0, 100]), undefined);
  assert.ok(segment[0][2] > 20);
  assert.ok(segment[1][2] < 100);
  hut.gameplay = {
    version: 1,
    collision: "none",
    doors: [],
    surfaces: supports.map((support, i) => ({
      id: `floor-${i}`,
      node: "building-999",
      polygon: support.points.map((p) => [p.x, p.y]),
      height: support.points.map((p) => p.z_top),
    })),
    lights: [{ ...region, node: "building-999" }],
  };
  for (const angle of [0, 90, 180, 270]) {
    document.groups[0]!.transform = { dx: 800, dy: 800, dz: 20, rot_deg: angle };
    const compiled = compileAssetGameplay(document, assets, [0, 0, 2000, 2000]);
    assert.equal(compiled.light_sectors!.length, 1, `rotation ${angle}`);
    assert.equal(compiled.light_sectors![0]!.ambience, 4);
  }
});

test("flat light fields retain receiving anchors and reject absent navigation", () => {
  const light: LightSector = {
    layer: 0,
    ambience: 4,
    polygon: {
      points: [
        [0, 0],
        [100, 0],
        [100, 50],
        [0, 50],
      ],
    },
  };
  const area: MotionArea = {
    is_lift: false,
    state_id: 0,
    flags: 0,
    skeleton_segments: [],
    obstacles: [],
    polygon: {
      points: [
        [10, 10],
        [30, 10],
        [30, 30],
        [10, 30],
      ],
    },
  };
  const { region } = recoverLightField(light, "flat", [], [area]);
  assert.deepEqual(
    region.polygon,
    light.polygon.points.map(([x, y]) => [x, y, 0]),
  );
  assert.equal(region.receivers!.length, 1);
  const [x, y, z] = region.receivers![0]!;
  assert.ok(x > 10 && x < 30 && y > 10 && y < 30);
  assert.equal(z, 0);
  assert.throws(() => recoverLightField(light, "flat", [], []), /no receiving anchors/);
});

test("light anchors use their movement area's receivers despite overlapping higher footprints", () => {
  const { hut } = assetCompilerFixture();
  const points: [number, number][] = [
    [10, 10],
    [30, 10],
    [30, 30],
    [10, 30],
  ];
  const supports: SightObstacle[] = [10, 20].map((height, sector) => ({
    ...hut.parts[0]!.obstacle_local_game!,
    projection_area: [sector, 1],
    points: points.map(([x, y]) => ({ x, y: y + height, z_bottom: 0, z_top: height })),
  }));
  const light: LightSector = { layer: 1, ambience: 4, polygon: { points } };
  const area: MotionArea = {
    is_lift: false,
    state_id: 0,
    flags: 0,
    skeleton_segments: [],
    obstacles: [],
    polygon: { points },
  };
  const { region } = recoverLightField(light, "overlap", supports, [area], [0]);
  assert.ok(region.receivers!.length);
  assert.ok(region.receivers!.every((point) => point[2] === 10));
  assert.deepEqual(
    region.polygon.map(([x, y, z]) => [x, y - z]),
    points,
  );
  assert.throws(
    () => recoverLightField(light, "overlap", supports, [area], []),
    /identities must match/,
  );
});

test("light fields retain the supporting plane for tiny clipped receiving triangles", () => {
  const { hut } = assetCompilerFixture();
  const support: SightObstacle = {
    ...hut.parts[0]!.obstacle_local_game!,
    projection_area: [0, 1],
    points: [
      [30 - 4 / 1048576, 30],
      [40, 20],
      [40, 40],
    ].map(([x, y]) => ({ x: x!, y: y! + 40, z_bottom: 0, z_top: 40 })),
  };
  const area: MotionArea = {
    is_lift: false,
    state_id: 0,
    flags: 0,
    skeleton_segments: [],
    obstacles: [],
    polygon: light.polygon,
  };
  const field = recoverLightField({ ...light, layer: 1 }, "small-intersection", [support], [area]);
  assert.deepEqual(
    field.region.polygon.map(([x, y, z]) => [x, y - z]),
    light.polygon.points,
  );
  assert.equal(field.region.receivers!.length, 1);
  assert.equal(field.region.receivers![0]![2], 40);
  const triangle = field.footprints.find((points) => points.length === 3)!;
  assert.ok(triangle);
  assert.throws(
    () => heightPlane(triangle.map(([x, y]) => [x, y - 40, 40])),
    /no nondegenerate height plane/,
  );
});

const light: LightSector = {
  layer: 0,
  ambience: 5,
  polygon: {
    points: [
      [10, 10],
      [30, 10],
      [30, 30],
      [10, 30],
    ],
  },
};
test("light ownership spans parts of one asset but rejects enclosed unowned gaps", () => {
  const rectangle = (x1: number, y1: number, x2: number, y2: number): [number, number][] => [
    [x1, y1],
    [x2, y1],
    [x2, y2],
    [x1, y2],
  ];
  assert.ok(
    containsLightPolygon(light.polygon.points, [
      rectangle(10, 10, 20, 30),
      rectangle(20, 10, 30, 30),
    ]),
  );
  assert.equal(
    containsLightPolygon(light.polygon.points, [
      rectangle(10, 10, 30, 15),
      rectangle(10, 25, 30, 30),
      rectangle(10, 15, 15, 25),
      rectangle(25, 15, 30, 25),
    ]),
    false,
  );
  assert.equal(containsLightPolygon(light.polygon.points, []), false);
});
test("ground light recovery localizes geometry and retains ambience without layer indices", () => {
  const plane = recoverLightPlane(light, []);
  assert.deepEqual(plane, [0, 0, 0]);
  const result = recoverLightRegion(light, "shadow", "wall", plane, ([x, y, z]) => [
    x - 10,
    y - 20,
    z,
  ]);
  assert.deepEqual(result, {
    id: "shadow",
    node: "wall",
    ambiences: 5,
    polygon: [
      [0, -10, 0],
      [20, -10, 0],
      [20, 10, 0],
      [0, 10, 0],
    ],
  });
});

test("multi-plane lights preserve projected union, holes, priority and local elevation", () => {
  const { hut } = assetCompilerFixture();
  const support = (left: number, right: number, z: number): SightObstacle => ({
    ...hut.parts[0]!.obstacle_local_game!,
    projection_area: [0, 0],
    points: [
      [left, left],
      [right, left],
      [right, right],
      [left, right],
    ].map(([x, y]) => ({
      x: x!,
      y: y! + z,
      z_top: z,
      z_bottom: 0,
    })),
  });
  const pieces = recoverLightRegions(
    light,
    "light",
    "wall",
    [support(15, 25, 40), support(18, 22, 60)],
    undefined,
    ([x, y, z]) => [x - 100, y - 200, z - 5],
  );
  assert.equal(new Set(pieces.map((p) => p.id)).size, pieces.length);
  const projected = pieces.map((p) => {
    assert.equal(p.node, "wall");
    assert.equal(p.ambiences, light.ambience);
    const points = p.polygon.map(([x, y, z]): [number, number] => [x + 100, y + 200 - (z + 5)]);
    const cx = points.reduce((s, p) => s + p[0], 0) / 3;
    const cy = points.reduce((s, p) => s + p[1], 0) / 3;
    const inside = (a: number, b: number) => cx > a && cx < b && cy > a && cy < b;
    const elevation = inside(18, 22) ? 60 : inside(15, 25) ? 40 : 0;
    assert.ok(p.polygon.every((p) => Math.abs(p[2] + 5 - elevation) < 1e-7));
    return [[...points, points[0]!]];
  });
  const union = fixedClipping.union(projected[0]!, ...projected.slice(1));
  const original = [[...light.polygon.points, light.polygon.points[0]!]];
  assert.equal(fixedClipping.difference(original, union).length, 0);
  assert.equal(fixedClipping.difference(union, original).length, 0);
  const area = pieces.reduce((sum, p) => {
    const [a, b, c] = p.polygon.map(([x, y, z]) => [x, y - z]);
    return (
      sum +
      Math.abs((b![0]! - a![0]!) * (c![1]! - a![1]!) - (b![1]! - a![1]!) * (c![0]! - a![0]!)) / 2
    );
  }, 0);
  assert.equal(area, 400, "pieces cover the contour once without filling holes twice");
  assert.throws(
    () =>
      recoverLightRegions(light, "light", "wall", [support(15.3, 25.3, 40)], undefined, (p) => p),
    /changes after integer quantization/,
  );
  const field = recoverLightField(
    light,
    "light",
    [support(15.3, 25.3, 40)],
    [
      {
        is_lift: false,
        state_id: 0,
        flags: 0,
        skeleton_segments: [],
        obstacles: [],
        polygon: light.polygon,
      },
    ],
  );
  assert.deepEqual(
    field.region.polygon.map(([x, y, z]) => [x, y - z]),
    light.polygon.points,
  );
  assert.equal(field.region.ambiences, light.ambience);
  assert.deepEqual(new Set(field.region.receivers!.map((p) => p[2])), new Set([0, 40]));
});

test("multi-plane recovery preserves single-plane contours and refuses missing elevated geometry", () => {
  assert.deepEqual(
    recoverLightRegions(light, "light", "wall", [], undefined, (p) => p),
    [recoverLightRegion(light, "light", "wall", [0, 0, 0], (p) => p)],
  );
  assert.throws(
    () => recoverLightRegions({ ...light, layer: 1 }, "light", "wall", [], undefined, (p) => p),
    /uncovered elevated/,
  );
});
test("light recovery uses projected elevation and refuses mixed or unsupported planes", () => {
  const { hut } = assetCompilerFixture();
  const support: SightObstacle = {
    ...hut.parts[0]!.obstacle_local_game!,
    projection_area: [0, 1],
    points: [
      [0, 0],
      [100, 0],
      [100, 100],
      [0, 100],
    ].map(([x, y]) => ({ x: x!, y: y! + 40, z_bottom: 0, z_top: 40 })),
  };
  const upper = { ...light, layer: 1 };
  assert.deepEqual(recoverLightPlane(upper, [support]), [0, 0, 40]);
  support.points[3]!.z_top += 0.01;
  support.points[3]!.y += 0.01;
  assert.deepEqual(recoverLightPlane(upper, [support]), [0, 0, 40]);
  assert.deepEqual(
    recoverLightRegion(upper, "light", "roof", [0, 0, 40], (p) => p).polygon[0],
    [10, 50, 40],
  );
  assert.throws(() => recoverLightPlane(upper, []), /uncovered elevated/);
  support.projection_area = [0, 0];
  support.points[1]!.x = support.points[2]!.x = 20;
  assert.throws(() => recoverLightPlane(light, [support]), /crosses receiving planes/);
});

for (const layer of [0, 1])
  test(`raised light contours on layer ${layer} can extend beyond navigation but not across receiving gaps`, () => {
    const { hut } = assetCompilerFixture();
    const support: SightObstacle = {
      ...hut.parts[0]!.obstacle_local_game!,
      projection_area: [0, layer],
      points: [
        [0, 0],
        [20, 0],
        [20, 100],
        [0, 100],
      ].map(([x, y]) => ({ x: x!, y: y! + 40, z_bottom: 0, z_top: 40 })),
    };
    const upper = { ...light, layer };
    const area: MotionArea = {
      is_lift: false,
      state_id: 0,
      flags: 0,
      skeleton_segments: [],
      obstacles: [],
      polygon: {
        points: [
          [0, 0],
          [20, 0],
          [20, 100],
          [0, 100],
        ],
      },
    };
    const plane = recoverLightPlane(upper, [support], [area]);
    assert.deepEqual(plane, [0, 0, 40]);
    assert.deepEqual(
      recoverLightRegion(upper, "shadow", "roof", plane, (point) => point).polygon,
      light.polygon.points.map(([x, y]) => [x, y + 40, 40]),
    );
    area.polygon.points[1]![0] = area.polygon.points[2]![0] = 40;
    assert.throws(
      () => recoverLightPlane(upper, [support], [area]),
      /uncovered elevated|crosses receiving/,
    );
    if (layer === 1) {
      const field = recoverLightField(upper, "field", [support], [area]);
      assert.deepEqual(
        field.region.polygon.map(([x, y, z]) => [x, y - z]),
        light.polygon.points,
      );
      assert.ok(field.region.receivers!.length > 0);
      assert.ok(
        field.region.receivers!.every(
          ([x, y, z]) => x >= 0 && x <= 20 && y - z >= 0 && y - z <= 100 && z === 40,
        ),
      );
      assert.deepEqual(
        field.footprints[0],
        field.region.polygon.map(([x, y]) => [x, y]),
      );
      const blockedArea: MotionArea = {
        ...area,
        obstacles: [
          {
            state_id: 0,
            polygon: {
              points: [
                [0, 0],
                [17, 0],
                [17, 100],
                [0, 100],
              ],
            },
          },
        ],
      };
      const unblockedField = recoverLightField(upper, "blocked-field", [support], [blockedArea]);
      assert.ok(unblockedField.region.receivers!.every(([x]) => x > 17 && x <= 20));
      assert.deepEqual(unblockedField.region.polygon, field.region.polygon);
      assert.throws(() => recoverLightField(upper, "field", [], [area]), /no receiving anchors/);
      const unsupportedArea = {
        ...area,
        polygon: {
          points: [
            [25, 0],
            [40, 0],
            [40, 100],
            [25, 100],
          ] as [number, number][],
        },
      };
      assert.throws(
        () => recoverLightField(upper, "field", [support], [unsupportedArea]),
        /no receiving anchors/,
      );
    }
    area.obstacles = [
      {
        state_id: 0,
        polygon: {
          points: [
            [20, 0],
            [40, 0],
            [40, 100],
            [20, 100],
          ],
        },
      },
    ];
    assert.deepEqual(recoverLightPlane(upper, [support], [area]), [0, 0, 40]);
    area.obstacles[0]!.state_id = 1;
    assert.throws(
      () => recoverLightPlane(upper, [support], [area]),
      /uncovered elevated|crosses receiving/,
    );
    if (layer === 0) assert.deepEqual(recoverLightPlane(upper, [], []), [0, 0, 0]);
    else assert.throws(() => recoverLightPlane(upper, [], []), /no receiving geometry/);
  });
