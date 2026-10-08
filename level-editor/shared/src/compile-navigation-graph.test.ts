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
