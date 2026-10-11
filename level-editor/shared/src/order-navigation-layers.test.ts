import test from "node:test";
import assert from "node:assert/strict";
import type { NavigationRegion } from "./assemble-navigation-regions.ts";
import type { HeightPlane } from "./gameplay-plane.ts";
import { orderNavigationLayers } from "./order-navigation-layers.ts";
import { assembleNavigationRegions, type NavigationPiece } from "./assemble-navigation-regions.ts";

function region(layer: number, plane: HeightPlane, x = 100): NavigationRegion {
  const polygon: [number, number][] = [
    [x, 100],
    [x + 100, 100],
    [x + 100, 200],
    [x, 200],
  ];
  return { layer, polygon, blockers: [], pieces: [{ layer, plane, polygon, blockers: [] }] };
}

test("click layers compare overlapping heights rather than a sloped plane's intercept", () => {
  const raised = region(0, [1, 0, -50]);
  const ground = region(1, [0, 0, 0]);
  const warnings: string[] = [];
  orderNavigationLayers([raised, ground], warnings);
  assert.deepEqual(warnings, []);
  assert.equal(ground.layer, 0);
  assert.equal(raised.layer, 1);
  assert.equal(raised.pieces[0]!.layer, 1);
});

test("joined receiver pieces keep one layer above the floor beneath every piece", () => {
  const raised = region(0, [1, 0, -50]);
  raised.pieces.push({ ...region(9, [-1, 0, 350], 200).pieces[0]!, layer: 0 });
  const ground = region(1, [0, 0, 0]);
  ground.pieces.push({ ...region(1, [0, 0, 0], 200).pieces[0]! });
  orderNavigationLayers([raised, ground], []);
  assert.equal(ground.layer, 0);
  assert.ok(raised.pieces.every((piece) => piece.layer === 1));
});

test("disjoint and coplanar receivers preserve their relative ordering", () => {
  const items = [region(0, [1, 0, -50]), region(1, [0, 0, 0], 300), region(2, [1, 0, -50])];
  const warnings: string[] = [];
  orderNavigationLayers(items, warnings);
  assert.deepEqual(
    items.map((item) => item.layer),
    [0, 1, 2],
  );
  assert.deepEqual(warnings, []);
});

test("crossing heights warn instead of dropping either receiving floor", () => {
  const items = [region(0, [1, 0, -150]), region(1, [0, 0, 0])];
  const warnings: string[] = [];
  orderNavigationLayers(items, warnings);
  assert.equal(warnings.length, 1);
  assert.match(warnings[0]!, /crossing receiving heights/);
  assert.deepEqual(
    items.map((item) => item.layer),
    [0, 1],
  );
  assert.ok(items.every((item) => item.pieces.length === 1));
});

test("an empty ground slot and reserved lift layers remain available", () => {
  const ordinary = region(42, [0, 0, 40]);
  const lift = { ...region(900, [0, 0, 0]), lift: "ladder" };
  orderNavigationLayers([ordinary, lift], []);
  assert.equal(ordinary.layer, 1);
  assert.equal(lift.layer, 900);
});

test("a crossing pair cannot put an unambiguous upper floor below it", () => {
  const upper = region(0, [0, 0, 200]);
  const ramp = region(1, [1, 0, -150]);
  const landing = region(2, [0, 0, 0]);
  const warnings: string[] = [];
  orderNavigationLayers([upper, ramp, landing], warnings);
  assert.deepEqual([ramp.layer, landing.layer, upper.layer], [0, 1, 2]);
  assert.equal(warnings.length, 1);
  assert.match(warnings[0]!, /^Navigation layers 0, 1:/);
  assert.match(warnings[0]!, /layers 1\/0 near projected \(150\.00, 150\.00\)/);
});

test("independent crossing pairs report only their own conflicting layers", () => {
  const items = [
    region(0, [1, 0, -150]),
    region(1, [0, 0, 0]),
    region(2, [1, 0, -450], 400),
    region(3, [0, 0, 0], 400),
  ];
  const warnings: string[] = [];
  orderNavigationLayers(items, warnings);
  assert.deepEqual(
    items.map((item) => item.layer),
    [0, 1, 2, 3],
  );
  assert.equal(warnings.length, 2);
  assert.match(warnings[0]!, /^Navigation layers 0, 1:/);
  assert.match(warnings[1]!, /^Navigation layers 2, 3:/);
});

test("a joined ramp starting on the ground receives a separate layer above another floor", () => {
  for (const reverse of [false, true]) {
    const piece = (
      name: string,
      layer: number,
      plane: HeightPlane,
      x0: number,
      x1: number,
    ): NavigationPiece => ({
      navigationRegion: name,
      layer,
      plane,
      blockers: [],
      polygon: [
        [x0, 100],
        [x1, 100],
        [x1, 200],
        [x0, 200],
      ],
    });
    const pieces = [
      piece("lower", 0, [0, 0, 0], 100, 300),
      piece("upper", 0, [0, 0, 0], 0, 50),
      piece("upper", 1, [2, 0, -100], 50, 100),
      piece("upper", 2, [0, 0, 100], 100, 200),
    ];
    if (reverse) pieces.reverse();
    const regions = assembleNavigationRegions(pieces, []);
    const warnings: string[] = [];
    orderNavigationLayers(regions, warnings);
    const lower = regions.find((r) => r.pieces[0]!.navigationRegion === "lower")!;
    const upper = regions.find((r) => r.pieces[0]!.navigationRegion === "upper")!;
    assert.equal(lower.layer, 0);
    assert.equal(upper.layer, 1);
    assert.ok(upper.pieces.every((p) => p.layer === upper.layer));
    assert.equal(upper.pieces.length, 3);
    assert.deepEqual(warnings, []);
  }
});
