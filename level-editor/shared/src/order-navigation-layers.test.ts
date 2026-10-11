import test from "node:test";
import assert from "node:assert/strict";
import type { NavigationRegion } from "./assemble-navigation-regions.ts";
import type { HeightPlane } from "./gameplay-plane.ts";
import { orderNavigationLayers } from "./order-navigation-layers.ts";

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
