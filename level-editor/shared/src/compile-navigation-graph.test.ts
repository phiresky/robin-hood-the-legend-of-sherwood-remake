import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { compileNavigationGraph, prepareCorridorStates } from "./compile-navigation-graph.ts";
import type { CompiledAssetGeometry } from "./asset-gameplay.ts";
import type { Point } from "./level.ts";

test("native routing fixture contains the graph generated from its motion geometry", () => {
  const motion: CompiledAssetGeometry["motion_data"] = JSON.parse(
    readFileSync(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/editor-navigation-graph.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileNavigationGraph(motion.layers), motion.graph_bytes);
  assert.ok(motion.graph_bytes.length > 100);
});

test("ordinary floors compile the same precise contour used by static collision", () => {
  const area: CompiledAssetGeometry["motion_data"]["layers"][number][number] = {
    is_lift: false,
    state_id: 0,
    flags: 0,
    obstacles: [],
    skeleton_segments: [],
    polygon: {
      points: [
        [0, 0],
        [100, 0],
        [100, 5],
        [5, 5],
        [5, 100],
        [0, 100],
      ],
    },
    precise_polygon: [
      [0, -0.4],
      [100, 0],
      [100, 5],
      [5, 5],
      [5, 100],
      [0, 100],
    ],
  };
  const ordinary = compileNavigationGraph([[area]]);
  const rounded = structuredClone(area);
  delete rounded.precise_polygon;
  assert.notDeepEqual(ordinary, compileNavigationGraph([[rounded]]));
});

test("large authored obstacle layouts export extended link indices", () => {
  const bytes = compileNavigationGraph([
    [
      {
        is_lift: false,
        state_id: 0,
        flags: 0,
        polygon: {
          points: [
            [0, 0],
            [7000, 0],
            [7000, 1000],
            [0, 1000],
          ],
        },
        skeleton_segments: [],
        obstacles: Array.from({ length: 64 }, (_, index) => {
          const x = 100 + index * 100;
          return {
            state_id: 0,
            polygon: {
              points: [
                [x, 400],
                [x + 20, 400],
                [x + 20, 420],
                [x, 420],
              ],
            },
          };
        }),
      },
    ],
    [],
  ]);
  assert.deepEqual(bytes.slice(0, 6), [255, 255, 1, 0, 1, 0]);
});

test("long returning corridors have graph entries inside the native source search margin", () => {
  const floor: Point[] = [
    [0, 0],
    [700, 0],
    [700, 60],
    [100, 60],
    [100, 140],
    [700, 140],
    [700, 200],
    [0, 200],
  ];
  const bytes = Uint8Array.from(
    compileNavigationGraph([
      [
        {
          is_lift: false,
          state_id: 0,
          flags: 0,
          polygon: { points: floor },
          skeleton_segments: [],
          obstacles: [],
        },
      ],
    ]),
  );
  const view = new DataView(bytes.buffer);
  let cursor = 0;
  const word = () => {
    const result = view.getUint16(cursor, true);
    cursor += 2;
    return result;
  };
  assert.equal(word(), 1);
  const half: Point = [view.getFloat32(cursor, true), view.getFloat32(cursor + 4, true)];
  cursor += 8;
  assert.equal(word(), 1); // layer
  assert.equal(word(), 1); // area
  assert.equal(word(), 1); // obstacle group
  const count = word();
  const docks: { position: Point; docking: Point }[] = [];
  for (let index = 0; index < count; index++) {
    assert.equal(word(), 1);
    const configuration = bytes[cursor++]!;
    const position: Point = [view.getInt16(cursor, true), view.getInt16(cursor + 2, true)];
    cursor += 16; // position, corner vectors, state
    const links = word();
    cursor += links * 2;
    const offsets: Point[] = [
      [-half[0], -half[1]],
      [half[0], -half[1]],
      [half[0], half[1]],
      [-half[0], half[1]],
    ];
    for (const [place, offset] of offsets.entries()) {
      if (configuration & (1 << place))
        docks.push({
          position,
          docking: [position[0] + offset[0], position[1] + offset[1]],
        });
    }
  }
  const allowed = prepareCorridorStates([], floor);
  const ends: Point[] = [
    [650, 30],
    [650, 170],
  ];
  assert.deepEqual(allowed(ends[0]!, ends[1]!), []);
  for (const source of ends) {
    assert.ok(
      docks.some(
        ({ position, docking }) =>
          position[0] >= 250 &&
          position[0] <= 1050 &&
          position[1] >= -370 &&
          position[1] <= 570 &&
          allowed(source, docking).length > 0,
      ),
      `no reachable graph entry inside the 400-unit search margin from ${source}`,
    );
  }
});
