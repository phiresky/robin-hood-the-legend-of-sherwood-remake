import test from "node:test";
import assert from "node:assert/strict";
import { authorWallWalkway } from "./author-wall-walkway.ts";
import { walkwayFixture } from "../test-fixtures/wall-walkway.ts";
import { compileMap } from "../../app/src/map-compile.ts";
import { validateAssetGameplay } from "../../shared/src/asset-gameplay.ts";
import { readFile } from "node:fs/promises";

test("dissolved wall caps preserve a continuous deck behind raised parapets", () => {
  const { asset } = walkwayFixture(),
    data = asset.gameplay!;
  validateAssetGameplay(data, asset);
  assert.equal(data.volumes!.length, 3);
  assert.equal(data.surfaces.length, 1);
  assert.equal(data.surfaces[0]!.polygon.length, 4);
  assert.ok(data.surfaces[0]!.polygon.every(([, y]) => y < 2));
  assert.ok(JSON.stringify(data).length < 3000);
});

test("walkway recipes reject a missing horizontal surface instead of inventing one", () => {
  assert.throws(
    () =>
      authorWallWalkway(
        [
          [
            [0, 0, 0],
            [100, 0, 40],
            [0, 20, 40],
          ],
        ],
        "body",
        { material: 2, opaque: true, walkwayHeight: 40 },
      ),
    /No horizontal walkway/,
  );
});

test("fractional source bounds retain exact repetition seams after cap simplification", () => {
  const shifted = walkwayFixture(0.00037),
    normal = walkwayFixture();
  const actual = compileMap(shifted.document, shifted.bounds, shifted.assets).descriptor
    .asset_geometry!.motion_data;
  const expected = compileMap(normal.document, normal.bounds, normal.assets).descriptor
    .asset_geometry!.motion_data;
  assert.deepEqual(actual, expected);
  assert.equal(actual.layers[1]!.length, 1);
});

test("curved wall triangles leave one ground region and one continuous walkway", async () => {
  const f = walkwayFixture();
  f.document.splines![0]!.curved = true;
  f.document.splines![0]!.points = [
    [100, 200, 0],
    [250, 200, 0],
    [400, 300, 0],
  ];
  const descriptor = compileMap(f.document, f.bounds, f.assets).descriptor;
  const geometry = descriptor.asset_geometry!;
  assert.deepEqual(
    geometry.motion_data.layers.map((layer) => layer.length),
    [1, 1, 0],
  );
  const expected = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-spline-curved-walkway.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(descriptor, expected);
});

test("repeated wall walkways export the native traversal fixture", async () => {
  const f = walkwayFixture();
  const expected = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-spline-walkway.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(f.document, f.bounds, f.assets).descriptor, expected);
});

test("curved walkways climbing a slope compile even when cap fragments collapse on the movement grid", () => {
  const f = walkwayFixture();
  f.document.splines![0]!.curved = true;
  f.document.splines![0]!.points = [
    [100, 200, 0],
    [250, 200, 0],
    [400, 300, 20],
  ];
  const geometry = compileMap(f.document, f.bounds, f.assets).descriptor.asset_geometry!;
  assert.ok(geometry.motion_data.layers.some((layer) => layer.length > 0));
  assert.ok(geometry.sight_obstacles.some((s) => s.points.some((p) => p.z_top > 40)));
});
