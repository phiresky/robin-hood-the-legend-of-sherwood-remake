import test from "node:test";
import assert from "node:assert/strict";
import {
  contractFlightRange,
  longFlightFamily,
  longTakeoffRibbons,
  type TakeoffReceiver,
} from "./long-jump-takeoff.ts";
import { integratedLongJumpTrajectory, type JumpEdge } from "./jump-clearance.ts";
import type { Vec3 } from "./scene.ts";

test("takeoff coverage reserves ground only beyond the receiving polygon", () => {
  const source: JumpEdge = { zone: "roof", a: [0, 80, 80], b: [0, 180, 80] };
  const receiver: TakeoffReceiver = {
    plane: [0, 0, 80],
    layer: 2,
    polygon: [
      [-10, 0],
      [10, 0],
      [10, 100],
      [-10, 100],
      [-10, 40],
      [-4, 40],
      [-4, 30],
      [-10, 30],
    ],
  };
  const ribbons = longTakeoffRibbons(source, receiver.plane, [receiver], []);
  assert.ok(ribbons.some((ribbon) => ribbon.points.every((point) => point[2] === 80)));
  let notch = false;
  for (const ribbon of ribbons.filter((item) => item.points.every((point) => point[2] === 0))) {
    for (const [x, y] of ribbon.points) {
      assert.ok(x <= -10 + 1e-7 || (x <= -4 + 1e-7 && y >= 30 - 1e-7 && y <= 40 + 1e-7));
      notch ||= x > -10 + 1e-7;
    }
    if (ribbon.points.some((point) => point[0] > -10 + 1e-7))
      assert.ok(ribbon.parameters!.every((t) => t >= 0.3 - 1e-7 && t <= 0.4 + 1e-7));
  }
  assert.ok(notch, "a narrow receiving gap must retain its own takeoff parameters");
});

test("takeoff follows adjacent receivers in its sector and layer, using highest top-bound priority", () => {
  const source: JumpEdge = { zone: "roof", a: [0, 80, 80], b: [0, 180, 80] };
  const rectangle = (low: number, high: number): [number, number][] => [
    [low, 0],
    [high, 0],
    [high, 100],
    [low, 100],
  ];
  const receivers: TakeoffReceiver[] = [
    { plane: [0, 0, 80], polygon: rectangle(-5, 5), sector: 2, layer: 1, maximumZ: 80 },
    { plane: [1, 0, 85], polygon: rectangle(-15, -5), sector: 2, layer: 1, maximumZ: 80 },
    { plane: [0, 0, 100], polygon: rectangle(-14, -10), sector: 2, layer: 1, maximumZ: 100 },
    { plane: [0, 0, 200], polygon: rectangle(-15, 5), sector: 3, layer: 1, maximumZ: 200 },
    { plane: [0, 0, 300], polygon: rectangle(-15, 5), sector: 2, layer: 2, maximumZ: 300 },
    { plane: [0, 0, 80], polygon: rectangle(-15, 5), sector: 4, layer: 1, maximumZ: 80 },
  ];
  assert.throws(() => longTakeoffRibbons(source, receivers[0]!.plane, receivers, []), /ambiguous/);
  const ribbons = longTakeoffRibbons(source, receivers[0]!.plane, receivers, [], {
    sector: 2,
    layer: 1,
  });
  assert.ok(ribbons.length > 0);
  const seen = new Set<string>();
  for (const ribbon of ribbons) {
    const x = ribbon.points.reduce((sum, point) => sum + point[0], 0) / 4;
    const z = ribbon.points.reduce((sum, point) => sum + point[2], 0) / 4;
    const kind = x > -5 ? "source" : x > -14 && x < -10 ? "upper" : "slope";
    const expected = kind === "source" ? 80 : kind === "upper" ? 100 : 85 + x;
    assert.ok(Math.abs(z - expected) < 1e-7, `${kind}: ${x}, ${z}, expected ${expected}`);
    seen.add(kind);
  }
  assert.deepEqual([...seen].sort(), ["slope", "source", "upper"]);
});

test("flight families cover native integration on both sides of frame-count boundaries", () => {
  for (const distance of [8, 16, 24, 40, 100]) {
    const targets: Vec3[] = [
      [distance, 0, 0],
      [distance + 100, 20, 30],
    ];
    const family = longFlightFamily([0, 0, 0], targets, 0.04);
    for (const amount of [-0.04, -0.02, 0, 0.02, 0.04]) {
      const actual = integratedLongJumpTrajectory([amount, 0, 0], targets);
      for (const point of actual.slice(1))
        assert.ok(
          family.some(
            (step) =>
              Math.hypot(...point.map((value, axis) => value - step.b[axis]!)) <=
              step.padding + 0.00001,
          ),
        );
    }
  }
});

test("sloped launch bounds retain native flights but never certify a short-order overshoot", () => {
  const starts: Vec3[] = [
    [-0.2, -0.1, -0.05],
    [0.2, 0.1, 0.05],
  ];
  const target: Vec3 = [100, 30, 60];
  const family = longFlightFamily([0, 0, 0], [target], 0.3);
  assert.ok(family.every((step) => step.contractions));
  for (const normal of [
    [1, -2, 3],
    [-3, 1, 2],
    [0.5, -0.25, -1],
  ]) {
    const dot = (point: Vec3) => point.reduce((sum, value, axis) => sum + value * normal[axis]!, 0);
    const bounds = [...starts, target].map(dot);
    for (const start of starts)
      for (const point of integratedLongJumpTrajectory(start, [target])) {
        const padding =
          Math.max(...family.map((step) => step.convexPadding)) *
          normal.reduce((sum, value) => sum + Math.abs(value), 0);
        assert.ok(dot(point) >= Math.min(...bounds) - padding);
        assert.ok(dot(point) <= Math.max(...bounds) + padding);
      }
  }
  const short = longFlightFamily([0, 0, 0], [[7, 0, 0]], 0.1);
  assert.ok(short.every((step) => step.contractions === undefined));
  assert.ok(integratedLongJumpTrajectory([0, 0, 0], [[7, 0, 0]])[1]![0] > 7);
});

test("later flight orders discard old launch heights while bounding native endpoints", () => {
  const starts: Vec3[] = [
    [-0.2, 0, 0],
    [0.2, 0, 0],
  ];
  const targets: Vec3[] = [
    [70, 0, 60],
    [140, 0, 100],
  ];
  const family = longFlightFamily([0, 0, 0], targets, 0.2);
  const later = family.filter((step) => step.contractions?.length === 2);
  assert.ok(later.length);
  for (const step of later) {
    const bounds = contractFlightRange(
      starts.map((p) => p[2]),
      step.contractions!,
      (p) => [p[2]],
    );
    assert.ok(bounds[0] > 40, "the launch floor must not remain in the second airborne order");
  }
  for (const start of starts) {
    const actual = integratedLongJumpTrajectory(start, targets);
    for (const normal of [
      [0, 0, 1],
      [0.2, -0.3, 1],
    ]) {
      const signed = (p: Vec3) => p.reduce((sum, value, axis) => sum + value * normal[axis]!, 0);
      for (const point of actual.slice(1, 3))
        assert.ok(
          later.some((step) => {
            const [low, high] = contractFlightRange(starts.map(signed), step.contractions!, (p) => [
              signed(p),
            ]);
            const error =
              step.convexPadding * normal.reduce((sum, value) => sum + Math.abs(value), 0);
            return signed(point) >= low - error && signed(point) <= high + error;
          }),
        );
    }
  }
});

test("final flight update binds directly to the receiving surface", () => {
  const final = longFlightFamily([0, 0, 0], [[40, 0, 0]], 0, [0, 0], true);
  assert.equal(Math.max(...final.filter((step) => !step.snap).map((step) => step.b[0])), 24);
  const intermediate = longFlightFamily(
    [0, 0, 0],
    [
      [40, 0, 0],
      [80, 0, 0],
    ],
    0,
    [0, 0],
    true,
  );
  assert.ok(intermediate.some((step) => !step.snap && step.b[0] === 32));

  const source: JumpEdge = { zone: "roof", a: [0, 80, 80], b: [0, 90, 80] };
  const ribbons = longTakeoffRibbons(
    source,
    [0, 0, 80],
    [],
    [[[100, 80, 80]]],
    undefined,
    [],
    undefined,
    true,
    [0, 0, 90],
  );
  const landings = ribbons.filter((ribbon) => ribbon.flightOrders && ribbon.padding === undefined);
  assert.ok(landings.length);
  for (const ribbon of landings) {
    assert.deepEqual(ribbon.points[0], ribbon.points[1]);
    assert.deepEqual(ribbon.points[2], ribbon.points[3]);
    assert.ok(ribbon.points.every((point) => point[0] === 100 && point[2] === 90));
  }
});

test("assisted departure lifts forty units in place across a sloped ledge", () => {
  const source: JumpEdge = { zone: "roof", a: [0, 80, 80], b: [0, 180, 80] };
  const receiver: TakeoffReceiver = {
    plane: [0, 0.1, 80],
    polygon: [
      [-20, 0],
      [10, 0],
      [10, 100],
      [-20, 100],
    ],
  };
  const ribbons = longTakeoffRibbons(
    source,
    receiver.plane,
    [receiver],
    [],
    undefined,
    [],
    undefined,
    true,
  );
  const points = ribbons.flatMap((ribbon) => ribbon.points);
  assert.ok(points.length);
  assert.ok(
    points.every((point) => point[0] === 0),
    "assistance must not reserve a forward run-up",
  );
  assert.equal(Math.min(...points.map((point) => point[2])), 80);
  assert.equal(Math.max(...points.map((point) => point[2])), 130);
  assert.ok(points.some((point) => point[1] === 80 && point[2] === 80));
  assert.ok(points.some((point) => point[1] === 80 && point[2] === 120));
});
