import test from "node:test";
import assert from "node:assert/strict";
import { assertSameData } from "../../shared/test-fixtures/assert-same-data.ts";
import { readFile } from "node:fs/promises";
import { gridTerrainCompilerFixture } from "../../shared/test-fixtures/grid-terrain.ts";
import { compileMap } from "./map-compile.ts";

test("edited terrain and river export matches the native elevation and traversal fixture", async () => {
  const document = gridTerrainCompilerFixture();
  const before = structuredClone(document);
  const compiled = compileMap(document, [0, 0, 220, 220], new Map());
  const expected = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/grid-terrain.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assertSameData(JSON.parse(JSON.stringify(compiled.descriptor)), expected);
  assert.deepEqual(document, before);
  assert.ok(document.terrain!.vertices.some((v) => v.position[0] > 220));
  for (const area of compiled.descriptor.asset_geometry!.motion_data.layers.flat())
    for (const [x, y] of area.polygon.points) assert.ok(x >= 0 && x <= 220 && y >= 0 && y <= 220);
});
