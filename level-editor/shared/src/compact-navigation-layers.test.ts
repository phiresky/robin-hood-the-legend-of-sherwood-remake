import assert from "node:assert/strict";
import test from "node:test";
import type { NavigationRegion } from "./assemble-navigation-regions.ts";
import { compactNavigationLayers } from "./compact-navigation-layers.ts";

function region(layer: number, lift?: string): NavigationRegion {
  const polygon: [number, number][] = [
    [0, 0],
    [10, 0],
    [10, 10],
  ];
  return {
    layer,
    lift,
    polygon,
    blockers: [],
    pieces: [{ layer, lift, polygon, blockers: [], plane: [0, 0, 0] }],
  };
}

test("compaction keeps separate receiving layers, stable region order, and final lift layer", () => {
  const regions = [region(0), region(42), region(42), region(1605), region(1804, "stairs")];
  const boundaries = regions.map((item) => item.polygon);
  assert.equal(compactNavigationLayers(regions), 3);
  assert.deepEqual(
    regions.map((item) => item.layer),
    [0, 1, 1, 2, 3],
  );
  assert.deepEqual(
    regions.map((item) => item.pieces[0]!.layer),
    [0, 1, 1, 2, 3],
  );
  assert.deepEqual(
    regions.map((item) => item.polygon),
    boundaries,
  );
});

test("empty ground and traversal layers retain their reserved roles", () => {
  const regions = [region(42)];
  assert.equal(compactNavigationLayers(regions), 2);
  assert.equal(regions[0]!.layer, 1);
  assert.equal(compactNavigationLayers([]), 1);
  const lift = [region(900, "stairs")];
  assert.equal(compactNavigationLayers(lift), 1);
  assert.equal(lift[0]!.layer, 1);
});

test("independent overlapping lifts receive separate layers while one lift keeps its pieces together", () => {
  const regions = [
    region(0),
    region(12, "stairs-a"),
    region(12, "stairs-b"),
    region(12, "stairs-a"),
  ];
  assert.equal(compactNavigationLayers(regions), 2);
  assert.deepEqual(
    regions.map((item) => [item.lift, item.layer]),
    [
      [undefined, 0],
      ["stairs-a", 1],
      ["stairs-a", 1],
      ["stairs-b", 2],
    ],
  );
  assert.ok(regions.every((item) => item.pieces.every((piece) => piece.layer === item.layer)));
});
