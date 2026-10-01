import test from "node:test";
import assert from "node:assert/strict";
import {
  sampleSpline,
  splineCurve,
  splineWidthAt,
  splineMaterialWeightsAt,
} from "./spline-sampling.ts";
import type { LevelSpline } from "./splines.ts";
const camera = { kind: "oblique-orthographic" as const, elevation_deg: 35 };
const path: LevelSpline = {
  id: "road",
  name: "Road",
  kind: "road",
  closed: false,
  width: 20,
  repeatLength: 100,
  points: [
    [0, 0, 0],
    [30, 0, 0],
    [500, 200, 0],
  ],
  pointWidths: [10, 50, 20],
};
test("point widths attach to curve sections rather than evenly spaced distance", () => {
  assert.equal(splineWidthAt(path, 0), 10);
  assert.equal(splineWidthAt(path, 0.5), 50);
  assert.equal(splineWidthAt(path, 1), 20);
  const samples = sampleSpline(path, camera);
  const curve = splineCurve(path, camera);
  for (const sample of samples) {
    assert.ok(sample.width >= 10 && sample.width <= 50);
    const parameter = curve.getUtoTmapping(sample.distance / curve.getLength(), 0);
    assert.ok(Math.abs(sample.width - splineWidthAt(path, parameter)) < 1e-8);
  }
  assert.ok(samples.find((s) => s.section === 1)!.distance < curve.getLength() / 2);
});
test("closed widths return continuously to the first control point", () => {
  const closed = { ...path, closed: true };
  assert.equal(splineWidthAt(closed, 1), 10);
  assert.equal(splineWidthAt(closed, 5 / 6), 15);
});
test("point material weights preserve a transition when adding a midpoint", () => {
  const two = {
    ...path,
    points: path.points.slice(0, 2),
    pointMaterials: ["path_dirt", "road_cobblestone"],
  };
  const middle = splineMaterialWeightsAt(two, 0.5);
  assert.deepEqual(middle, { path_dirt: 0.5, road_cobblestone: 0.5 });
  const three = {
    ...path,
    pointMaterials: ["path_dirt", "path_dirt", "road_cobblestone"],
    pointMaterialMixes: [null, middle, null],
  };
  assert.deepEqual(splineMaterialWeightsAt(three, 0.25), splineMaterialWeightsAt(two, 0.25));
  assert.deepEqual(splineMaterialWeightsAt(three, 0.75), splineMaterialWeightsAt(two, 0.75));
});

test("straight walls interpolate controls exactly and measure uneven sections by distance", () => {
  const wall: LevelSpline = {
    ...path,
    kind: "wall",
    curved: false,
    points: [
      [0, 0, 0],
      [30, 0, 0],
      [30, 100, 0],
    ],
  };
  const curve = splineCurve(wall, camera);
  const secondLength = 100 / Math.sin((camera.elevation_deg * Math.PI) / 180);
  assert.ok(Math.abs(curve.getLength() - 30 - secondLength) < 1e-9);
  assert.ok(curve.getPoint(0.5).distanceTo(curve.getPoint(0).setX(30)) < 1e-9);
  assert.ok(Math.abs(curve.getPointAt(15 / curve.getLength()).x - 15) < 1e-9);
  assert.equal(curve.getPointAt(0.8).x, 30);
  const closed = splineCurve({ ...wall, closed: true }, camera);
  assert.ok(closed.getPoint(0).distanceTo(closed.getPoint(1)) < 1e-9);
  assert.ok(Math.abs(closed.getLength() - curve.getLength() - Math.hypot(30, secondLength)) < 1e-9);
  assert.notEqual(splineCurve({ ...wall, curved: undefined }, camera).getPoint(0.25).y, 0);
});

test("straight paths sample every control and use matching corner joins for all kinds", () => {
  for (const kind of ["wall", "road", "river"] as const) {
    const straight = { ...path, kind, curved: false };
    const curve = splineCurve(straight, camera);
    const samples = sampleSpline(straight, camera);
    assert.ok(samples.some((s) => s.position.distanceTo(curve.getPoint(0.5)) < 1e-9));
    for (const s of samples) {
      assert.ok(s.position.distanceTo(curve.getPoint((s.section + s.fraction) / 2)) < 1e-9);
      assert.ok(s.lateralScale >= 1 && s.lateralScale <= 4);
    }
    assert.equal(samples.at(-1)!.distance, curve.getLength());
  }
});
