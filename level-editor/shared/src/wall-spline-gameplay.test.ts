import test from "node:test";
import assert from "node:assert/strict";
import { wallSplineFixture } from "../test-fixtures/wall-spline.ts";
import { wallSplineGameplay } from "./wall-spline-gameplay.ts";
import { compileAssetGameplay } from "./compile-asset-gameplay.ts";
import { validateAssetGameplay } from "./asset-gameplay.ts";

test("wall collision follows moved paths, crops repeats and participates in terrain navigation", () => {
  const { document, assets, bounds } = wallSplineFixture();
  const first = compileAssetGameplay(document, assets, bounds);
  assert.equal(first.warnings?.length ?? 0, 0);
  assert.ok(first.sight_obstacles.filter((s) => s.solid).length >= 6);
  const points = first.sight_obstacles.filter((s) => s.solid).flatMap((s) => s.points);
  assert.equal(Math.min(...points.map((p) => p.x)), 100);
  assert.equal(Math.max(...points.map((p) => p.x)), 400);
  document.splines![0]!.points = [
    [130, 260, 0],
    [345, 260, 0],
  ];
  const second = compileAssetGameplay(document, assets, bounds);
  const changed = second.sight_obstacles.filter((s) => s.solid).flatMap((s) => s.points);
  assert.equal(Math.min(...changed.map((p) => p.x)), 130);
  assert.equal(Math.max(...changed.map((p) => p.x)), 345);
  assert.ok(changed.every((p) => p.y > 250 && p.y < 270));
  assert.notDeepEqual(first.motion_data, second.motion_data);
});

test("a raised wall keeps its underpass and opaque and solid flags independent", () => {
  const { document, assets, asset, bounds } = wallSplineFixture();
  asset.gameplay!.volumes![0]!.shape.opaque = false;
  document.splines![0]!.points = [
    [100, 200, 80],
    [400, 200, 80],
  ];
  const raised = compileAssetGameplay(document, assets, bounds);
  const empty = compileAssetGameplay({ ...document, splines: [] }, assets, bounds);
  assert.deepEqual(raised.motion_data, empty.motion_data);
  assert.ok(
    raised.sight_obstacles
      .filter((s) => s.solid)
      .every((s) => !s.opaque && s.points.every((p) => p.z_bottom === 80)),
  );
});

test("calibrated wall-top surfaces and tower corners survive deformation", () => {
  const { document, asset, assets, bounds } = wallSplineFixture();
  const shape = asset.gameplay!.volumes![0]!.shape;
  asset.gameplay!.surfaces = [
    {
      id: "walkway",
      node: "body",
      polygon: shape.points.map((p) => [p.x, p.y]),
      height: shape.points[0]!.z_top,
    },
  ];
  document.splines![0]!.points = [
    [100, 100, 0],
    [300, 100, 0],
    [300, 300, 0],
  ];
  document.splines![0]!.cornerAsset = "wall";
  document.splines![0]!.cornerScale = 1.2;
  const walls = wallSplineGameplay(document, assets, false);
  assert.deepEqual(walls.warnings, []);
  for (const descriptor of walls.descriptors)
    validateAssetGameplay(descriptor.gameplay, descriptor);
  const result = compileAssetGameplay(document, assets, bounds);
  assert.ok(result.sight_obstacles.some((s) => s.points.some((p) => p.z_top > 39)));
  assert.ok(result.motion_data.layers.length > 1);
});

test("missing calibration warns without removing other wall spans or corrupting input", () => {
  const { document, asset, assets } = wallSplineFixture();
  document.splines!.push({ ...document.splines![0]!, id: "missing", asset: "unknown" });
  const before = JSON.stringify([document, asset]);
  assert.throws(() => wallSplineGameplay(document, assets, false), /calibration/);
  const result = wallSplineGameplay(document, assets, true);
  assert.equal(result.warnings.length, 1);
  assert.ok(result.descriptors[0]!.gameplay!.volumes!.length);
  assert.equal(JSON.stringify([document, asset]), before);
});
