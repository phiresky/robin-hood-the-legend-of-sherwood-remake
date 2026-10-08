import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { compileNavigationGraph } from "./compile-navigation-graph.ts";
import type { CompiledAssetGeometry } from "./asset-gameplay.ts";

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
