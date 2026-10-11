import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import type { CompiledAssetGeometry } from "./asset-gameplay.ts";
import type { Point } from "./level.ts";
import { prepareCorridorStates } from "./compile-navigation-graph.ts";
import {
  compileLiftPassageStates,
  compileOrdinaryPassageStates,
} from "./compile-passage-states.ts";
import {
  ordinaryPassageBarrierCompilerFixture,
  alternatingPostureCompilerFixture,
} from "../test-fixtures/asset-gameplay.ts";
import { compileAssetGameplay } from "./compile-asset-gameplay.ts";

const rectangle: Point[] = [
  [3, -10],
  [4, -10],
  [4, 10],
  [3, 10],
];
const obstacle = (state_id: number) => ({ state_id, polygon: { points: rectangle } });

test("posture seams retain distinct openings for alternate asset states", () => {
  const { document, assets } = alternatingPostureCompilerFixture();
  const geometry = compileAssetGameplay(document, assets, [0, 0, 2000, 2000]);
  const states = compileOrdinaryPassageStates(geometry);
  assert.equal(geometry.doors.length, 4, "two usable openings per copied seam");
  for (const pair of [states.slice(0, 2), states.slice(2)]) {
    assert.deepEqual(
      pair.map((requirements) => requirements[0]!.allowed_states[0]!).sort((a, b) => a - b),
      [1, 2],
    );
    assert.ok(pair.every((requirements) => requirements.every((r) => r.allowed_states.length > 0)));
  }
});

test("rotated copied posture seams remain usable in each alternate state", () => {
  for (const rotation of [0, 37, 90, 180]) {
    const { document, assets } = alternatingPostureCompilerFixture();
    document.size = [3000, 2000];
    for (const group of document.groups)
      group.transform = {
        dx: group.id.startsWith("copy/") ? 1600 : 900,
        dy: 900,
        dz: 40,
        rot_deg: rotation,
      };
    const geometry = compileAssetGameplay(document, assets, [0, 0, 3000, 2000]);
    const states = compileOrdinaryPassageStates(geometry);
    const copies = new Map<string, typeof states>();
    geometry.doors.forEach((door, index) => {
      const key = [door.sector_in, door.sector_out].sort((a, b) => a - b).join("/");
      const entries = copies.get(key) ?? [];
      entries.push(states[index]!);
      copies.set(key, entries);
    });
    assert.equal(copies.size, 2);
    for (const entries of copies.values())
      for (const state of [1, 2]) {
        assert.ok(
          entries.some((requirements) =>
            requirements.every((r) => r.allowed_states.some((mask) => (state & mask) === mask)),
          ),
          `rotation ${rotation}, state ${state}`,
        );
      }
  }
});

test("ordinary passage states bind changing blockers to their own copied receiving areas", () => {
  const { document, assets } = ordinaryPassageBarrierCompilerFixture();
  const geometry = compileAssetGameplay(document, assets, [0, 0, 2000, 2000]);
  const before = structuredClone(geometry);
  const states = compileOrdinaryPassageStates(geometry);
  assert.equal(states.length, 2);
  assert.ok(states.every((requirements) => requirements.length === 1));
  assert.deepEqual(
    states.map((requirements) => requirements[0]!.allowed_states),
    [[1], [1]],
  );
  assert.notEqual(states[0]![0]!.area, states[1]![0]!.area);
  assert.deepEqual(geometry, before);
  geometry.doors[0]!.sector_out = 65534;
  assert.throws(() => compileOrdinaryPassageStates(geometry), /missing out area/);
});

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
