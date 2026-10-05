import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

// Explicitly selected authoring candidates only. Publication requires mesh and
// placed actor checks; this never repairs a scene silently during compilation.
const [asset, ...ids] = process.argv.slice(2);
assert.ok(asset && ids.length, "Provide an asset and selected stair lift IDs");
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
const entry = index.find((entry) => entry.id === asset);
assert.ok(entry, asset);
const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
const descriptorSha256 = createHash("sha256").update(bytes).digest("hex");
assert.equal(descriptorSha256, entry.descriptor_sha256);
const descriptor = JSON.parse(bytes);
const gameplay = structuredClone(descriptor.gameplay);
const changes = [];
for (const id of ids) {
  const lift = gameplay.lifts.find((lift) => lift.id === id);
  assert.equal(lift?.type, 1, `${id}: not a stair`);
  const floor = gameplay.surfaces.find((surface) => surface.id === lift.surface);
  assert.ok(floor && !floor.holes?.length, `${id}: requires separate hole review`);
  const before = structuredClone(floor);
  const heights = floor.polygon.map((_, i) =>
    Array.isArray(floor.height) ? floor.height[i] : floor.height,
  );
  const plane = heightPlane(floor.polygon.map(([x, y], i) => [x, y, heights[i]]));
  const length = Math.hypot(plane[0], plane[1]);
  assert.ok(length > 1e-6);
  const seat = (point, z) => {
    const t = (z - planeHeight(plane, point)) / (length * length);
    return [point[0] + t * plane[0], point[1] + t * plane[1]];
  };
  const endpoints = [...new Set(lift.doors.map((door) => door.outside[2]))];
  // Only end-adjacent vertices move. Bent side boundaries retain their
  // intermediate heights and cannot be flattened into an endpoint plane.
  floor.height = [...heights];
  floor.polygon = floor.polygon.map((point, i) => {
    const z = endpoints.reduce((a, b) =>
      Math.abs(heights[i] - a) < Math.abs(heights[i] - b) ? a : b,
    );
    if (Math.abs(heights[i] - z) / length > 2) return point;
    floor.height[i] = z;
    return seat(point, z);
  });
  floor.preserveMovementPrecision = true;
  changes.push({ surface: floor.id, before, after: structuredClone(floor) });
  const adjusted = new Set();
  for (const door of lift.doors) {
    const landings = gameplay.surfaces.filter(
      (surface) =>
        surface !== floor &&
        (Array.isArray(surface.height) ? surface.height : [surface.height]).every(
          (z) => Math.abs(z - door.outside[2]) < 1e-4,
        ) &&
        pointInGameplayPolygon(door.outside, surface.polygon, true) &&
        !(surface.holes ?? []).some((hole) => pointInGameplayPolygon(door.outside, hole, true)),
    );
    assert.equal(landings.length, 1, `${door.id}: requires external or ambiguous landing review`);
    const landing = landings[0];
    const a = planeHeight(plane, door.outside),
      b = planeHeight(plane, door.inside);
    const t = (door.outside[2] - a) / (b - a);
    assert.ok(t >= 0 && t <= 1, `${door.id}: landing is outside the approach`);
    const oldMiddle = door.middle;
    door.middle = [
      door.outside[0] + t * (door.inside[0] - door.outside[0]),
      door.outside[1] + t * (door.inside[1] - door.outside[1]),
      door.outside[2],
    ];
    assert.ok(
      Math.hypot(...door.middle.map((v, i) => v - oldMiddle[i])) < 3,
      `${door.id}: excessive midpoint correction`,
    );
    assert.ok(
      pointInGameplayPolygon(door.middle, floor.polygon, true),
      `${door.id}: midpoint still unsupported`,
    );
    changes.push({ door: door.id, before: oldMiddle, after: door.middle, landing: landing.id });
    if (adjusted.has(landing.id)) continue;
    const sideways = (point) => (-plane[1] * point[0] + plane[0] * point[1]) / length;
    const seam = floor.polygon.filter((_, i) => Math.abs(floor.height[i] - door.outside[2]) < 1e-4);
    assert.ok(seam.length >= 2, `${door.id}: missing complete end edge`);
    const low = Math.min(...seam.map(sideways)),
      high = Math.max(...seam.map(sideways));
    const oldLanding = structuredClone(landing);
    const vertices = new Set();
    // A receiving edge can extend beyond both sides of a narrower stair.
    // Adjust the complete near-coplanar edge, not only vertices inside its span.
    landing.polygon.forEach((point, i) => {
      const j = (i + 1) % landing.polygon.length;
      const next = landing.polygon[j];
      if (
        [point, next].every(
          (p) => Math.abs(planeHeight(plane, p) - door.outside[2]) / length <= 2,
        ) &&
        Math.min(high, Math.max(sideways(point), sideways(next))) >
          Math.max(low, Math.min(sideways(point), sideways(next)))
      ) {
        vertices.add(i);
        vertices.add(j);
      }
    });
    assert.ok(vertices.size >= 2, `${door.id}: no matching landing edge`);
    landing.polygon = landing.polygon.map((point, i) =>
      vertices.has(i) ? seat(point, door.outside[2]) : point,
    );
    landing.preserveMovementPrecision = true;
    changes.push({ surface: landing.id, before: oldLanding, after: structuredClone(landing) });
    adjusted.add(landing.id);
  }
  gameplay.movementClearances ??= [];
  const clearance = {
    id: `${floor.id}-seam-clearance`,
    node: floor.node,
    polygon: structuredClone(floor.polygon),
    height: [...floor.height],
    holes: [],
  };
  assert.ok(!gameplay.movementClearances.some((value) => value.id === clearance.id));
  gameplay.movementClearances.push(clearance);
  changes.push({ clearance: clearance.id, after: clearance });
}
validateAssetGameplay(gameplay, descriptor);
const output = await fs.mkdtemp("work/map-compile/local-stair-seams-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify([{ asset, descriptorSha256, gameplay }]));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify(
    { scope: "unpublished local stair seam candidate", asset, descriptorSha256, changes },
    null,
    2,
  ),
);
console.log(JSON.stringify({ output, asset, lifts: ids, changes: changes.length }));
