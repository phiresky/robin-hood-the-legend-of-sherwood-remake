import test from "node:test";
import assert from "node:assert/strict";
import type { LevelSpline, MapCamera } from "@rle/shared";
import { insertSplinePoint, nearestSplineSection } from "./spline-insertion.ts";
const camera: MapCamera = { kind: "oblique-orthographic", elevation_deg: 35 };
const path: LevelSpline = {
  id: "road",
  name: "Road",
  kind: "road",
  closed: false,
  points: [
    [0, 0, 10],
    [100, 0, 30],
    [100, 100, 50],
  ],
  width: 20,
  repeatLength: 100,
  pointWidths: [20, 40, 60],
  pointHeightOffsets: [0, 10, 20],
  pointMaterials: ["path_dirt", "path_gravel", "path_sand"],
  cornerDisabled: [1, 2],
};
test("inserting at clicked ground preserves point data and interpolates the section", () => {
  const result = insertSplinePoint(path, 0, 0.25, [25, -20, 999]);
  assert.deepEqual(result.points[1], [25, -20, 15]);
  assert.deepEqual(result.pointWidths, [20, 25, 40, 60]);
  assert.deepEqual(result.pointHeightOffsets, [0, 2.5, 10, 20]);
  assert.deepEqual(result.pointMaterialMixes?.[1], { path_dirt: 0.75, path_gravel: 0.25 });
  assert.deepEqual(result.cornerDisabled, [2, 3]);
  assert.equal(path.points.length, 3);
});
test("nearest section follows the curve and supports the closing seam", () => {
  assert.equal(nearestSplineSection(path, camera, [30, -5, 0]).section, 0);
  assert.equal(nearestSplineSection(path, camera, [105, 65, 0]).section, 1);
  const closed = { ...path, closed: true };
  assert.equal(nearestSplineSection(closed, camera, [40, 45, 0]).section, 2);
  const result = insertSplinePoint(closed, 2, 0.5);
  assert.deepEqual(result.points[3], [50, 50, 30]);
  assert.deepEqual(result.pointMaterialMixes?.[3], { path_sand: 0.5, path_dirt: 0.5 });
});
test("insertion preserves existing mixtures and rejects invalid sections or the point limit", () => {
  const mixed = { ...path, pointMaterialMixes: [{ path_dirt: 0.5, path_sand: 0.5 }, null, null] };
  const result = insertSplinePoint(mixed, 0, 0.5);
  assert.deepEqual(result.pointMaterialMixes?.[1], {
    path_dirt: 0.25,
    path_sand: 0.25,
    path_gravel: 0.5,
  });
  assert.throws(() => insertSplinePoint(path, 2, 0.5));
  assert.throws(() =>
    insertSplinePoint(
      { ...path, points: Array.from({ length: 256 }, (): [number, number, number] => [0, 0, 0]) },
      0,
      0.5,
    ),
  );
});
