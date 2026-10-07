import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { compileMap } from "../app/src/map-compile.ts";
import { wallSplineFixture, wallMaterialFixture, wallAutomaticLightFixture } from "../shared/test-fixtures/wall-spline.ts";
const require = createRequire(new URL("../shared/package.json", import.meta.url));
const clipping = require("polygon-clipping");
const output = await fs.mkdtemp("work/map-compile/wall-fixture-coverage-");
const reports = [];
const canonical = (value) => JSON.stringify(value, (_, v) =>
  v && typeof v === "object" && !Array.isArray(v)
    ? Object.fromEntries(Object.entries(v).sort(([a], [b]) => a.localeCompare(b))) : v);
function partition(document) {
  const retained = structuredClone(document);
  const receivers = new Map();
  retained.asset_geometry.sight_obstacles = retained.asset_geometry.sight_obstacles.filter((obstacle) => {
    if (obstacle.solid || obstacle.opaque || !Array.isArray(obstacle.projection_area)) return true;
    const height = obstacle.points[0].z_top;
    if (!obstacle.points.every((p) => p.z_top === height && p.z_bottom === height)) return true;
    const { points, ...properties } = obstacle;
    const key = canonical({ properties, height });
    const polygons = receivers.get(key) ?? [];
    polygons.push([points.map((p) => [p.x, p.y])]);
    receivers.set(key, polygons);
    return false;
  });
  return { retained, receivers };
}
for (const [name, fixture] of [
  ["asset-spline-wall", wallSplineFixture],
  ["asset-spline-material", wallMaterialFixture],
  ["asset-spline-auto-light", wallAutomaticLightFixture],
]) {
  const filename = `${name}.level.json`;
  const expected = JSON.parse(await fs.readFile(`../crates/robin_engine/tests/fixtures/${filename}`));
  const f = fixture();
  const actual = compileMap(f.document, f.bounds, f.assets).descriptor;
  const old = partition(expected), current = partition(actual);
  assert.deepEqual(current.retained, old.retained, `${name}: non-receiver gameplay changed`);
  assert.deepEqual([...current.receivers.keys()].sort(), [...old.receivers.keys()].sort());
  for (const [key, polygons] of old.receivers) {
    const before = clipping.union(...polygons), after = clipping.union(...current.receivers.get(key));
    assert.deepEqual(clipping.xor(before, after), [], `${name}: receiver coverage changed: ${key}`);
  }
  await fs.writeFile(`${output}/${filename}`, JSON.stringify(actual));
  reports.push({ name, oldObstacles: expected.asset_geometry.sight_obstacles.length,
    newObstacles: actual.asset_geometry.sight_obstacles.length, equivalentReceiverGroups: old.receivers.size });
}
await fs.writeFile(`${output}/report.json`, JSON.stringify(reports, null, 2));
console.log(JSON.stringify({ output, reports }, null, 2));
