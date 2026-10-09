import test from "node:test";
import assert from "node:assert/strict";
import {
  createJumpClearance,
  boundedLongJumpTrajectory,
  integratedJumpTrajectory,
  integratedLongJumpTrajectory,
  longJumpTrajectory,
  type JumpEdge,
} from "./jump-clearance.ts";
import { assembleJumpSegments, type PlacedJumpSegment } from "./assemble-jump-segments.ts";
import type { SightObstacle } from "./level.ts";
import type { Vec3 } from "./scene.ts";

test("long-flight bounds cover launch variation across airborne frame thresholds", () => {
  for (const distance of [7.99, 8, 15.99, 16, 16.01, 24, 40, 100]) {
    const targets: Vec3[] = [
      [distance, 0, 0],
      [distance + 100, 20, 30],
    ];
    const bounds = boundedLongJumpTrajectory([0, 0, 0], targets, 0.04);
    for (const direction of [
      [1, 0, 0],
      [0, 1, 0],
      [0, 0, 1],
      [0, Math.SQRT1_2, Math.SQRT1_2],
    ])
      for (const amount of [-0.04, -0.02, 0, 0.02, 0.04]) {
        const start = direction.map((axis) => axis * amount) as Vec3;
        const path = integratedLongJumpTrajectory(start, targets);
        for (const [index, bound] of bounds.entries()) {
          // Endpoint error bounds also enclose the straight fixed-step segment.
          const error = Math.hypot(
            ...path[index + 1]!.map((value, axis) => value - bound.b[axis]!),
          );
          assert.ok(
            error <= bound.padding + 0.00001,
            `${distance}/${amount}/${index}: ${error} > ${bound.padding}`,
          );
        }
      }
  }
  assert.throws(
    () => boundedLongJumpTrajectory([0, 0, 0], [[0.01, 0, 0]], 0.02),
    /zero-length order/,
  );
});

const edges: [JumpEdge, JumpEdge] = [
  { zone: "left", a: [0, 100, 0], b: [0, 0, 0] },
  { zone: "right", a: [60, 0, 0], b: [60, 100, 0] },
];

test("long-flight clearance includes an upright takeoff that stays at the ledge", () => {
  const pair: [JumpEdge, JumpEdge] = [
    { zone: "low", a: [380, 370, 0], b: [380, 330, 0] },
    { zone: "high", a: [420, 410, 80], b: [420, 450, 80] },
  ];
  // A recorded native airborne position after a stationary takeoff animation.
  const obstacle = wall(383.063038, 373.503468, 0.04, 0.04, 6.466904, 6.506904);
  assert.ok(createJumpClearance([obstacle])(pair, true).some(([a, b]) => a <= 0 && b >= 0));
});

test("sword flight clearance covers an intermediate twelve-unit takeoff", () => {
  const pair: [JumpEdge, JumpEdge] = [
    { zone: "low", a: [1524, 774, 0], b: [1594, 774, 0] },
    { zone: "high", a: [1594, 894, 80], b: [1524, 894, 80] },
  ];
  const obstacle = wall(1523.98, 798.836934, 0.04, 0.04, 9.503642, 9.543642);
  assert.ok(createJumpClearance([obstacle])(pair, true).some(([a, b]) => a <= 0 && b >= 0));
});

test("vertical airborne integration uses native speeds and final snapping", () => {
  assert.deepEqual(integratedJumpTrajectory([0, 0, 0], [[100, 0, 0]], "up"), [
    [0, 0, 0],
    [75.00000762939453, 0, 0],
    [100, 0, 0],
  ]);
  assert.deepEqual(integratedJumpTrajectory([0, 0, 0], [[100, 0, 0]], "down"), [
    [0, 0, 0],
    [80, 0, 0],
    [100, 0, 0],
  ]);
  assert.deepEqual(integratedJumpTrajectory([0, 0, 0], [[5, 0, 0]], "up"), [
    [0, 0, 0],
    [15, 0, 0],
    [5, 0, 0],
  ]);
});
function wall(
  x: number,
  y: number,
  width: number,
  length: number,
  bottom = -10,
  top = 100,
): SightObstacle {
  return {
    points: [
      [x, y],
      [x + width, y],
      [x + width, y + length],
      [x, y + length],
    ].map(([x, y]) => ({ x: x!, y: y!, z_bottom: bottom, z_top: top })),
    solid: true,
    opaque: true,
    mouse: true,
    show_shadow_polygon: false,
    default_material: 0,
    projection_area: null,
    material_indices: [],
  };
}
function segments(): PlacedJumpSegment[] {
  return edges.map((edge) => ({
    id: edge.zone,
    edge,
    long: true,
    attachment: { maxGap: 80, maxRise: 20, maxDrop: 20, minOverlap: 10 },
  }));
}

test("vertical clearance covers airborne steps, receiver binding and the landing lift", () => {
  const climbing: [JumpEdge, JumpEdge] = [
    edges[0],
    { zone: "upper", a: [36, 100, 100], b: [36, 200, 100] },
  ];
  assert.deepEqual(createJumpClearance([])(climbing, false), []);
  // Complete-profile native samples at the midpoint, relative to the lower ledge.
  for (const [x, y, z] of [
    [3.082763671875, 64.6798095703125, 40],
    [12.3310546875, 108.71923828125, 40],
    [21, 210, 100],
    [21, 210, 160],
    [36, 150, 100],
    [36, 150, 50],
    [29.87005615234375, 132.972412109375, 41.48619842529297],
  ]) {
    const blocked = createJumpClearance([
      wall(x! - 0.02, y! - 0.02, 0.04, 0.04, z! - 0.02, z! + 0.02),
    ])(climbing, false);
    assert.ok(
      blocked.some(([a, b]) => a <= 0.5 && b >= 0.5),
      `uncovered native position ${x},${y},${z}`,
    );
  }
  assert.deepEqual(createJumpClearance([wall(20, 210, 2, 2, 170, 180)])(climbing, false), []);
});

test("geometric vertical connections trim blockers and retain independent usable spans", () => {
  const placed = segments();
  placed[1]!.edge = { zone: "upper", a: [36, 100, 100], b: [36, 200, 100] };
  for (const segment of placed) {
    segment.long = false;
    segment.attachment = { maxGap: 80, maxRise: 110, maxDrop: 110, minOverlap: 10 };
  }
  assert.equal(assembleJumpSegments(placed, createJumpClearance([])).pairs.length, 1);
  const blocker = wall(20, 200, 2, 10, 150, 165);
  const result = assembleJumpSegments(placed, createJumpClearance([blocker]));
  assert.equal(result.pairs.length, 2);
  for (const pair of result.pairs)
    assert.deepEqual(createJumpClearance([blocker])(pair.edges as [JumpEdge, JumpEdge], false), []);
});

test("automatic vertical clearance follows the receiving plane during the landing lift", () => {
  const climbing: [JumpEdge, JumpEdge] = [
    edges[0],
    {
      zone: "upper",
      a: [36, 100, 100],
      b: [36, 200, 100],
    },
  ];
  const receiver = wall(30, 90, 20, 120);
  receiver.solid = false;
  receiver.projection_area = [1, 1];
  receiver.projection_plane = [
    [36, 100, 100],
    [46, 101, 101],
    [36, 200, 100],
  ];
  assert.deepEqual(createJumpClearance([receiver])(climbing, false), []);
  const obstruction = wall(20.98, 208.48, 0.04, 0.04, 158.48, 158.52);
  const blocked = createJumpClearance([receiver, obstruction])(climbing, false);
  assert.ok(blocked.some(([a, b]) => a <= 0.5 && b >= 0.5));
  assert.doesNotThrow(() => createJumpClearance([receiver])(climbing, true));
  const competing = structuredClone(receiver);
  competing.projection_plane![1]![1] += 1;
  competing.projection_plane![1]![2] += 1;
  assert.throws(
    () => createJumpClearance([receiver, competing])(climbing, false),
    /ambiguous receiving planes/,
  );

  const shallow = structuredClone(climbing);
  for (const point of [shallow[1].a, shallow[1].b]) {
    point[1] -= 20;
    point[2] -= 20;
  }
  const lowerReceiver = structuredClone(receiver);
  for (const point of lowerReceiver.points) {
    point.y -= 20;
    point.z_top -= 20;
    point.z_bottom -= 20;
  }
  for (const point of lowerReceiver.projection_plane!) {
    point[1] -= 20;
    point[2] -= 20;
  }
  assert.deepEqual(createJumpClearance([lowerReceiver])(shallow, false), []);
});

test("flight clearance intersects the whole span, including thin off-centre obstacles", () => {
  const blocked = createJumpClearance([wall(30, 21, 0.1, 0.2)])(edges, true);
  assert.equal(blocked.length, 1);
  assert.ok(blocked[0]![0] < 0.79 && blocked[0]![1] >= 0.79);
  const result = assembleJumpSegments(segments(), createJumpClearance([wall(30, 20, 1, 10)]));
  assert.equal(result.pairs.length, 2);
  assert.equal(result.warnings.length, 1);
  assert.deepEqual(
    result.pairs.map((pair) => pair.edges[0]!.a[1]),
    [100, 19],
  );
  assert.deepEqual(
    result.pairs.map((pair) => pair.edges[0]!.b[1]),
    [31, 0],
  );
});

test("inactive physical volumes do not remove usable jump spans", () => {
  const blocker = wall(30, -100, 1, 300);
  assert.equal(assembleJumpSegments(segments(), createJumpClearance([blocker])).pairs.length, 0);
  blocker.initial_active = false;
  const result = assembleJumpSegments(segments(), createJumpClearance([blocker]));
  assert.equal(result.pairs.length, 1);
  assert.deepEqual(result.pairs[0]!.edges, edges);
});

test("full walls omit the connection and non-solid or distant volumes do not", () => {
  const full = wall(30, -100, 1, 300);
  const result = assembleJumpSegments(segments(), createJumpClearance([full]));
  assert.equal(result.pairs.length, 0);
  assert.match(result.warnings[0]!, /0 usable span/);
  for (const obstacle of [
    { ...full, solid: false },
    wall(200, 0, 5, 100),
    wall(30, 0, 1, 100, 100, 120),
    wall(30, 0, 1, 100, -100, -10),
  ])
    assert.deepEqual(createJumpClearance([obstacle])(edges, true), []);
});

test("a low obstacle is cleared by the arc but an obstacle intersecting its apex blocks it", () => {
  assert.deepEqual(createJumpClearance([wall(30, -100, 1, 300, 0, 5)])(edges, true), []);
  assert.deepEqual(createJumpClearance([wall(30, -100, 1, 300, 20, 25)])(edges, true), [[0, 1]]);
  const path = longJumpTrajectory([15, 0, 0], [60, 0, 0]);
  assert.deepEqual(path.at(-1), [60, 0, 0]);
  assert.ok(path.some((p) => p[2] > 20));
});

test("sword-fighting flight rejects low barriers below the ordinary jump arc", () => {
  const obstruction = wall(30, 20, 1, 10, -1, 5);
  const blocked = createJumpClearance([obstruction])(edges, true);
  assert.ok(blocked.length > 0);
  const result = assembleJumpSegments(segments(), createJumpClearance([obstruction]));
  assert.equal(result.pairs.length, 2);
  assert.ok(result.warnings.some((warning) => warning.includes("obstruct the flight")));
  for (const pair of result.pairs)
    assert.deepEqual(
      createJumpClearance([obstruction])(pair.edges as [JumpEdge, JumpEdge], true),
      [],
    );
});

test("airborne order integration checks the space between the arc and direct flight", () => {
  const start: [number, number, number] = [15, 0, 0];
  const path = integratedLongJumpTrajectory(start, longJumpTrajectory(start, [60, 0, 0]).slice(1));
  assert.deepEqual(path, [
    start,
    [23.8948974609375, 0, 13.299652099609375],
    [31.237524032592773, 0, 16.475473403930664],
    [52.06294250488281, 0, 4.546437740325928],
    [60, 0, 0],
  ]);
  // At x=29 the ideal arc is above z=20 and sword flight stays at zero.
  const obstruction = wall(29, 20, 0.1, 10, 15, 18);
  assert.ok(createJumpClearance([obstruction])(edges, true).length > 0);
  const result = assembleJumpSegments(segments(), createJumpClearance([obstruction]));
  assert.equal(result.pairs.length, 2);
  for (const pair of result.pairs)
    assert.deepEqual(
      createJumpClearance([obstruction])(pair.edges as [JumpEdge, JumpEdge], true),
      [],
    );
});

test("takeoff motion is checked and floor contact is allowed", () => {
  assert.deepEqual(createJumpClearance([wall(5, -100, 1, 300)])(edges, true), [[0, 1]]);
  assert.deepEqual(createJumpClearance([wall(-100, -100, 300, 300, -10, 0)])(edges, true), []);
});

test("assisted departure checks the elevated path before the ordinary takeoff point", () => {
  // The ordinary arc starts fifteen units away; assisted takeoff rises in place.
  const obstruction = wall(-1, 20, 2, 10, 35, 45);
  assert.ok(createJumpClearance([obstruction])(edges, true).length > 0);
  const result = assembleJumpSegments(segments(), createJumpClearance([obstruction]));
  assert.equal(result.pairs.length, 2);
  for (const pair of result.pairs)
    assert.deepEqual(
      createJumpClearance([obstruction])(pair.edges as [JumpEdge, JumpEdge], true),
      [],
    );
});

test("short sword flights check the fixed-step overshoot beyond the landing", () => {
  const close: [JumpEdge, JumpEdge] = [
    edges[0],
    {
      zone: "right",
      a: [20, 0, 0],
      b: [20, 100, 0],
    },
  ];
  // The five-unit flight still advances eight units before snapping to the ledge.
  assert.deepEqual(integratedLongJumpTrajectory([15, 0, 0], [[20, 0, 0]]), [
    [15, 0, 0],
    [23, 0, 0],
    [20, 0, 0],
  ]);
  assert.deepEqual(createJumpClearance([wall(22, -10, 1, 120, -1, 1)])(close, true), [[0, 1]]);
});

test("authored body height and radius protect headroom and edge clearance", () => {
  const ceiling = createJumpClearance([wall(30, -100, 1, 300, 50, 60)]);
  assert.deepEqual(ceiling(edges, true), []);
  assert.deepEqual(ceiling(edges, true, { radius: 4, height: 40 }), [[0, 1]]);
  const side = createJumpClearance([wall(30, 101, 1, 5)]);
  assert.deepEqual(side(edges, true), []);
  assert.ok(side(edges, true, { radius: 4, height: 40 }).length > 0);
});

test("unsupported automatic flight geometry warns without breaking the export", () => {
  const placed = segments();
  placed[0]!.edge = { ...placed[0]!.edge, b: [0, 10, 10] };
  const result = assembleJumpSegments(placed, createJumpClearance([]));
  assert.equal(result.pairs.length, 0);
  assert.match(result.warnings[0]!, /Sloped jump edges/);
});

test("integer jump heights stay above fractional supporting surfaces", () => {
  const placed = segments();
  for (const segment of placed)
    segment.edge = {
      ...segment.edge,
      a: [segment.edge.a[0], segment.edge.a[1] + 10.3, 10.3],
      b: [segment.edge.b[0], segment.edge.b[1] + 10.3, 10.3],
    };
  const result = assembleJumpSegments(
    placed,
    createJumpClearance([wall(-10, -100, 20, 300, 0, 10.3), wall(50, -100, 20, 300, 0, 10.3)]),
  );
  assert.equal(result.pairs.length, 1);
  assert.equal(result.pairs[0]!.edges[0]!.a[2], 11);
});

test("a gap shorter than takeoff cannot create a backward flight", () => {
  const placed = segments();
  placed[1]!.edge = { zone: "right", a: [10, 0, 0], b: [10, 100, 0] };
  const result = assembleJumpSegments(placed, createJumpClearance([]));
  assert.equal(result.pairs.length, 0);
  assert.match(result.warnings[0]!, /15-unit takeoff/);
});
