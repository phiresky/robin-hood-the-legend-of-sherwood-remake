import assert from "node:assert/strict";
import test from "node:test";
import { crossesGroundFootprint, selectPanelRoute } from "./select-panel-route.mjs";

const box = (left, top, right, bottom, height = 0) => ({
  points: [
    [left, top],
    [right, top],
    [right, bottom],
    [left, bottom],
  ].map(([x, y]) => ({
    x,
    y: y + height,
    z_bottom: height,
  })),
});

test("a blocked horizontal approach still permits a clear vertical diagnostic route", () => {
  const target = box(0, 0, 100, 100);
  const prop = box(-30, -30, -5, 130);
  const route = selectPanelRoute(target, [prop], 0);
  assert.ok(Math.abs(route.start[0] - route.end[0]) < 1e-8);
  assert.ok(route.start[1] < 0 && route.end[1] > 100);
  assert.equal(crossesGroundFootprint(route.start, route.end, target, 0), true);
  assert.equal(crossesGroundFootprint(route.start, route.end, prop, 0), false);
});

test("diagnostics reject a fully obstructed passage instead of relaxing clearance", () => {
  assert.throws(
    () => selectPanelRoute(box(0, 0, 100, 100), [box(-30, -30, 130, 130)], 0),
    /No conservative passage/,
  );
});

test("boundary probes distinguish replaced ground props from overhead panels", () => {
  const start = [-15, 50],
    end = [115, 50];
  assert.equal(crossesGroundFootprint(start, end, box(0, 0, 100, 100), 0), true);
  assert.equal(crossesGroundFootprint(start, end, box(0, 0, 100, 100, 100), 0), false);
  assert.equal(crossesGroundFootprint(start, end, box(0, 150, 100, 250), 0), false);
  assert.equal(crossesGroundFootprint(start, end, box(0, 0, 100, 100, 40), 40), true);
});
