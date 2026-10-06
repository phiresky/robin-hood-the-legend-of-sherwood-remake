import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { fixedPolygonBoolean } from "../shared/src/fixed-polygon-boolean.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

// Reproject reviewed ground openings onto a raised approach, restricted to the
// stair contact strip. This is an authoring candidate, never an export fallback.
const [source] = process.argv.slice(2);
assert.ok(source);
const edits = JSON.parse(await fs.readFile(`${source}/edits.json`, "utf8"));
assert.equal(edits.length, 1);
const edit = edits[0];
assert.equal(edit.asset, "york-east-riverside-southern-wall-stair");
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
const entry = index.find((e) => e.id === edit.asset);
const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
assert.equal(createHash("sha256").update(bytes).digest("hex"), edit.descriptorSha256);
const descriptor = JSON.parse(bytes),
  gameplay = edit.gameplay;
const lift = gameplay.lifts.find((l) => l.id === "building-268-lift");
const door = lift.doors.find((d) => d.type === 5);
const surface = gameplay.surfaces.find((s) => s.id === lift.surface);
const plane = heightPlane(surface.polygon.map((p, i) => [...p, surface.height[i]]));
const z = door.outside[2];
for (const floor of gameplay.surfaces) {
  const receiver = floor.projectionMaterials;
  if (!receiver) continue;
  receiver.footprint = floor.polygon.map((p, i) => [...p, floor.height[i]]);
  let best = 0;
  for (let i = 1; i < receiver.footprint.length - 1; i++) {
    const [a, b, c] = [receiver.footprint[0], receiver.footprint[i], receiver.footprint[i + 1]];
    const area = Math.abs((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]));
    if (area > best) {
      best = area;
      receiver.planePoints = [a, b, c];
    }
  }
  assert.ok(best > 1e-6);
  heightPlane(receiver.planePoints);
  receiver.priorityHeight = Math.max(...floor.height);
}
const seam = surface.polygon.filter((p) => Math.abs(planeHeight(plane, p) - z) < 1e-5);
assert.equal(seam.length, 2);
const length = Math.hypot(plane[0], plane[1]);
const sign = Math.sign(planeHeight(plane, door.outside) - z);
const offset = plane.slice(0, 2).map((v) => (v * sign * 30) / length);
const strip = [seam[0], seam[1], ...[seam[1], seam[0]].map((p) => p.map((v, i) => v + offset[i]))];
const added = [];
for (const node of ["building-282--component-wall-stair", "building-285"]) {
  const flight = {
    id: `${node}-reviewed-stair-flight`,
    node,
    polygon: structuredClone(surface.polygon),
    height: [...surface.height],
    holes: [],
    preserveMovementPrecision: true,
  };
  gameplay.movementClearances.push(flight);
  added.push(flight);
  const ground = gameplay.movementClearances.find((c) => c.id === `${node}-clearance-3-0`);
  assert.ok(ground && ground.height.every((h) => h === 0));
  const project = (ring) => ring.map(([x, y]) => [x, y + z]);
  const polygons = fixedPolygonBoolean(
    "intersection",
    [strip],
    [[project(ground.polygon), ...(ground.holes ?? []).map(project)]],
  );
  assert.ok(polygons.length);
  for (const [i, polygon] of polygons.entries()) {
    const ring = polygon[0].slice(0, -1);
    const clearance = {
      id: `${node}-raised-approach-${i}`,
      node,
      polygon: ring,
      height: ring.map(() => z),
      holes: polygon.slice(1).map((r) => r.slice(0, -1)),
      preserveMovementPrecision: true,
    };
    gameplay.movementClearances.push(clearance);
    added.push(clearance);
  }
}
validateAssetGameplay(gameplay, descriptor);
const review = JSON.parse(await fs.readFile(`${source}/review.json`, "utf8"));
review.changes.push({ raisedApproachClearances: added, requiresMeshAndNativeReview: true });
const output = await fs.mkdtemp("work/map-compile/raised-stair-approach-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(`${output}/review.json`, JSON.stringify(review));
console.log(JSON.stringify({ output, added: added.length }));
