import test from "node:test";
import assert from "node:assert/strict";
import { createTerrainGrid, type Level3D } from "@rle/shared";
import { terrainContours } from "./terrain-contours.ts";

function slope(): Level3D {
  const terrain = createTerrainGrid([0, 0, 128, 128], 128);
  for (const vertex of terrain.vertices) vertex.position[2] = vertex.position[0] - 64;
  return {
    version: 1,
    map: "contours",
    sceneAssets: [],
    objects: [],
    groups: [],
    camera: { kind: "oblique-orthographic", elevation_deg: 35 },
    terrain,
  };
}

test("contours intersect slopes at positive, zero and negative game heights", () => {
  const segments = terrainContours(slope());
  assert.deepEqual(
    [...new Set(segments.map(([a]) => a[2]))].sort((a, b) => a - b),
    [-64, -32, 0, 32, 64],
  );
  for (const [a, b] of segments) {
    assert.equal(a[2], b[2]);
    assert.equal(a[0], a[2] + 64);
    assert.equal(b[0], b[2] + 64);
    assert.notDeepEqual(a, b);
  }
  for (const height of [-64, -32, 0, 32, 64]) {
    const length = segments
      .filter(([a]) => a[2] === height)
      .reduce((sum, [a, b]) => sum + Math.abs(a[1] - b[1]), 0);
    assert.equal(length, 128, "Each contour spans the slope without gaps or duplicate edges");
  }
});

test("flat terrain has no artificial triangle grid; shared ridge edges appear once", () => {
  const document = slope();
  document.terrain = createTerrainGrid([0, 0, 128, 128], 64, 32);
  assert.deepEqual(terrainContours(document), []);
  for (const vertex of document.terrain.vertices)
    vertex.position[2] = vertex.position[0] === 64 ? 32 : 0;
  const ridge = terrainContours(document).filter(([a]) => a[2] === 32);
  assert.equal(ridge.length, 2);
  assert.equal(
    ridge.reduce((sum, [a, b]) => sum + Math.abs(a[1] - b[1]), 0),
    128,
  );
  assert.throws(() => terrainContours(document, 0), /interval/);
});
