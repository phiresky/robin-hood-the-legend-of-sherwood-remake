import test from "node:test";
import assert from "node:assert/strict";
import {
  createJumpClearance,
  integratedLongJumpTrajectory,
  longJumpTrajectory,
  type JumpEdge,
} from "./jump-clearance.ts";
import { assembleJumpSegments, type PlacedJumpSegment } from "./assemble-jump-segments.ts";
import type { SightObstacle } from "./level.ts";

const edges: [JumpEdge, JumpEdge] = [
  { zone: "left", a: [0, 100, 0], b: [0, 0, 0] },
  { zone: "right", a: [60, 0, 0], b: [60, 100, 0] },
];
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
