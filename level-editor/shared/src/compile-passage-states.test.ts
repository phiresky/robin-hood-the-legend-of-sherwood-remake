import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import type { CompiledAssetGeometry } from "./asset-gameplay.ts";
import type { Point } from "./level.ts";
import { prepareCorridorStates } from "./compile-navigation-graph.ts";
import { compileLiftPassageStates } from "./compile-passage-states.ts";

const rectangle: Point[] = [
  [3, -10],
  [4, -10],
  [4, 10],
  [3, 10],
];
const obstacle = (state_id: number) => ({ state_id, polygon: { points: rectangle } });

test("passage clearance handles permanent, alternate and independent obstacle states", () => {
  const states = (...ids: number[]) => prepareCorridorStates(ids.map(obstacle))([0, 0], [10, 0]);
  assert.deepEqual(states(), [0]);
  assert.deepEqual(states(0), []);
  assert.deepEqual(states(1), [2]);
  assert.deepEqual(states(2), [1]);
  assert.deepEqual(states(1, 2), []);
  assert.deepEqual(states(2, 8), [5]);
  assert.deepEqual(states(10), [1, 4]);
  assert.deepEqual(states(0x80000000), [0x40000000]);
  assert.deepEqual(states(0x40000000), [0x80000000]);
});

test("animated passage checks obstacles across a floor edge without blocking the edge itself", () => {
  const floor: Point[] = [
    [0, -20],
    [20, -20],
    [20, 20],
    [0, 20],
  ];
  assert.deepEqual(prepareCorridorStates([], floor)([0, 0], [10, 0]), []);
  assert.deepEqual(prepareCorridorStates([])([0, 0], [10, 0]), [0]);
  assert.deepEqual(prepareCorridorStates([obstacle(2)])([0, 0], [10, 0]), [1]);
  assert.deepEqual(prepareCorridorStates([obstacle(2)])([10, 0], [0, 0]), [1]);
  assert.deepEqual(prepareCorridorStates([obstacle(2)])([20, 0], [30, 0]), [0]);
});

test("fresh climb exports detect the entry blocker in every rotated placement", () => {
  const fixtures: { asset_geometry: CompiledAssetGeometry }[] = JSON.parse(
    readFileSync(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-climb-entrance-barriers.levels.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.equal(fixtures.length, 24);
  for (const [index, { asset_geometry: geometry }] of fixtures.entries()) {
    const before = JSON.stringify(geometry);
    const conditions = compileLiftPassageStates(geometry);
    assert.deepEqual(
      conditions[0]![index < 12 ? 0 : 1],
      [{ layer: 2, area: 0, allowed_states: [1] }],
      `placement ${index}`,
    );
    assert.deepEqual(conditions[0]![index < 12 ? 1 : 0], [], `opposite entrance ${index}`);
    assert.equal(
      JSON.stringify(geometry),
      before,
      "preparation must not alter authored/exported geometry",
    );
  }
});
