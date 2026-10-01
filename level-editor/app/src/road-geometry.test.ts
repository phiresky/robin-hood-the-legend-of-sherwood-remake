import test from "node:test";
import assert from "node:assert/strict";
import { roadGeometry, previewRoadGeometry } from "./road-geometry.ts";
import { terrainHeightAt, type TerrainGrid } from "../../shared/src/authored-terrain.ts";
import type { Level3D, LevelSpline } from "@rle/shared";
const camera = { kind: "oblique-orthographic", elevation_deg: 35 } as const;
const cosine = Math.cos((35 * Math.PI) / 180),
  sine = Math.sin((35 * Math.PI) / 180);
function fixture(): Level3D {
  const xs = [0, 49, 50, 51, 100];
  const terrain: TerrainGrid = { version: 1, spacing: 100, vertices: [], cells: [] };
  for (const y of [0, 100])
    for (const x of xs)
      terrain.vertices.push({ id: `${x}/${y}`, position: [x, y, x === 50 ? 30 : 0] });
  for (let x = 0; x < 4; x++)
    terrain.cells.push({ id: `${x}`, vertices: [x, x + 1, x + 6, x + 5], material: "grass_short" });
  return {
    version: 1,
    map: "test",
    size: null,
    camera,
    terrain,
    objects: [],
    groups: [],
    sceneAssets: [],
  };
}
const path: LevelSpline = {
  id: "road",
  kind: "road",
  points: [
    [10, 50, 7],
    [90, 50, 7],
  ],
  width: 30,
  repeatLength: 64,
  closed: false,
};
test("roads follow a two-pixel ridge between sample stations and preserve UVs", () => {
  const document = fixture(),
    geometry = roadGeometry({ ...path, pointHeightOffsets: [2, 2] }, camera, document);
  const p = geometry.getAttribute("position"),
    uv = geometry.getAttribute("uv");
  let peak = false;
  for (let i = 0; i < p.count; i += 3) {
    // Interior samples detect faces that bridge a ridge despite matching at their vertices.
    for (const weights of [
      [1, 0, 0],
      [0.2, 0.3, 0.5],
      [1 / 3, 1 / 3, 1 / 3],
    ]) {
      let x = 0,
        y = 0,
        z = 0;
      for (let k = 0; k < 3; k++) {
        x += weights[k]! * p.getX(i + k);
        y -= weights[k]! * p.getY(i + k) * sine;
        z += weights[k]! * (p.getZ(i + k) - 0.8) * cosine;
      }
      const expected = terrainHeightAt(document, x, y)! + 2;
      assert.ok(Math.abs(expected - z) < 2e-4, `${x}/${y}: expected ${expected}, got ${z}`);
      if (z > 31.99) peak = true;
    }
  }
  assert.ok(peak);
  for (let i = 0; i < p.count; i++) {
    assert.ok(Math.abs(uv.getY(i) - (p.getX(i) - 10) / 64) < 1e-6);
    assert.ok(uv.getX(i) >= -1e-6 && uv.getX(i) <= 1 + 1e-6);
  }
  geometry.dispose();
});
test("outside terrain roads retain authored heights and total area", () => {
  const geometry = roadGeometry(
    {
      ...path,
      points: [
        [-20, 50, 7],
        [120, 50, 7],
      ],
    },
    camera,
    fixture(),
  );
  const p = geometry.getAttribute("position");
  let area = 0,
    outside = false;
  for (let i = 0; i < p.count; i += 3) {
    area +=
      Math.abs(
        (p.getX(i + 1) - p.getX(i)) * (p.getY(i + 2) - p.getY(i)) -
          (p.getY(i + 1) - p.getY(i)) * (p.getX(i + 2) - p.getX(i)),
      ) / 2;
    for (let k = 0; k < 3; k++)
      if (p.getX(i + k) < -1 || p.getX(i + k) > 101) {
        outside = true;
        assert.ok(Math.abs((p.getZ(i + k) - 0.8) * cosine - 7) < 1e-5);
      }
  }
  assert.ok(outside);
  assert.ok(Math.abs(area - 140 * 30) < 0.01, `${area}`);
  geometry.dispose();
});

test("drag previews sample terrain and offsets with bounded geometry, then restore narrow ridges", () => {
  const document = fixture();
  const preview = previewRoadGeometry({ ...path, pointHeightOffsets: [2, 2] }, camera, document);
  const p = preview.getAttribute("position");
  for (let i = 0; i < p.count; i++) {
    const expected = terrainHeightAt(document, p.getX(i), -p.getY(i) * sine)! + 2;
    assert.ok(Math.abs((p.getZ(i) - 0.8) * cosine - expected) < 0.0001);
  }
  const wide = previewRoadGeometry(
    {
      ...path,
      width: 10000,
      points: [
        [-10000, 0, 7],
        [10000, 0, 7],
      ],
    },
    camera,
    document,
  );
  assert.ok(wide.getAttribute("position").count <= 257 * 9);
  assert.ok(Math.abs((wide.getAttribute("position").getZ(0) - 0.8) * cosine - 7) < 0.0001);
  const exact = roadGeometry(path, camera, document);
  assert.ok(Array.from(exact.getAttribute("position").array).every(Number.isFinite));
  preview.dispose();
  wide.dispose();
  exact.dispose();
});
