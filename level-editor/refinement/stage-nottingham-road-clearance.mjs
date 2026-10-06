import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

// Seat the asset-owned foundation clearance on its corrected stair entrance.
// The clearance removes only this asset's collision; it supplies no new floor.
const [source] = process.argv.slice(2);
assert.ok(source, "Provide the reviewed stair seam stage");
const edits = JSON.parse(await fs.readFile(`${source}/edits.json`, "utf8"));
assert.equal(edits.length, 1);
const contacts = {
  "nottingham-southeast-road-props": {
    node: "building-057",
    clearance: "building-057-clearance-0-1",
    ground: "ground-section-0-0",
    hole: [
      [1670, 1698],
      [1660, 1690],
      [1636, 1696],
      [1623, 1681],
      [1643, 1665],
      [1702, 1647],
      [1730, 1681],
    ],
    issue:
      "Corrected stair and foundation contact retain incomplete mesh coverage: 753/829 floor samples hit the mesh, with a 2.504-unit maximum edge discrepancy. Complete rendered actor integration remains unverified.",
  },
  "nottingham-southwest-prison-road-props": {
    node: "building-483",
    clearance: "building-483-clearance-3-1",
    ground: "ground-section-3-0",
    hole: [
      [728, 2322],
      [665, 2337],
      [630, 2338],
      [619, 2324],
      [646, 2300],
      [693, 2281],
    ],
    issue:
      "Corrected prison-road stair has complete sampled flight and landing mesh support; upright clearance collision follows the low deck. Complete rendered actor integration remains unverified.",
  },
};
const contact = contacts[edits[0].asset];
assert.ok(contact, "No reviewed foundation contact for this asset");
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
const entry = index.find((e) => e.id === edits[0].asset);
assert.ok(entry);
const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
assert.equal(createHash("sha256").update(bytes).digest("hex"), edits[0].descriptorSha256);
const descriptor = JSON.parse(bytes);
const gameplay = edits[0].gameplay;
const review = JSON.parse(await fs.readFile(`${source}/review.json`, "utf8"));
const floor = gameplay.surfaces.find((s) => s.id === `${contact.node}-walk-0`);
const lift = gameplay.lifts.find((l) => l.id === `${contact.node}-lift`);
const clearance = gameplay.movementClearances.find((c) => c.id === contact.clearance);
assert.ok(floor && lift && clearance);
const plane = heightPlane(floor.polygon.map(([x, y], i) => [x, y, floor.height[i]]));
const height = lift.doors[0].outside[2];
assert.ok(clearance.height.every((z) => Math.abs(z - height) < 1e-8));
const before = structuredClone(clearance);
const shifts = [];
for (const i of [0, 1]) {
  const p = clearance.polygon[i];
  const distance = (planeHeight(plane, p) - height) / Math.hypot(plane[0], plane[1]);
  assert.ok(Math.abs(distance) < 2, "Foundation contact needs broader review");
  const t = distance / Math.hypot(plane[0], plane[1]);
  clearance.polygon[i] = [p[0] - t * plane[0], p[1] - t * plane[1]];
  shifts.push(Math.abs(distance));
}
review.changes.push({ clearance: clearance.id, before, after: structuredClone(clearance), shifts });
// Low timber prevents upright passage beneath the deck. Keep this collision
// with the placed asset instead of leaving a permanent cut in the terrain.
assert.equal(gameplay.movementBlockers, undefined);
assert.equal(gameplay.movementSolids, undefined);
assert.equal(gameplay.volumes, undefined);
gameplay.volumes = descriptor.parts.map((part) => {
  const shape = structuredClone(part.obstacle_local_game);
  assert.ok(shape?.solid);
  delete shape.projection_area;
  delete shape.material_indices;
  shape.opaque = false;
  shape.mouse = false;
  shape.show_shadow_polygon = false;
  return { id: `${part.node}-upright-clearance`, node: part.node, movementHeadroom: 80, shape };
});
gameplay.movementSolids = gameplay.volumes.map((v) => v.id);
gameplay.draft.issues.push(contact.issue);
validateAssetGameplay(gameplay, descriptor);
const output = await fs.mkdtemp("work/map-compile/nottingham-road-clearance-");
await fs.mkdir(`${output}/placement`);
await fs.writeFile(`${output}/placement/edits.json`, JSON.stringify(edits, null, 2));
await fs.writeFile(`${output}/placement/review.json`, JSON.stringify(review, null, 2));
const terrainEntry = index.find((e) => e.id === "nottingham-terrain");
assert.ok(terrainEntry);
const terrainBytes = await fs.readFile(`library/3d-assets/${terrainEntry.descriptor}`);
assert.equal(
  createHash("sha256").update(terrainBytes).digest("hex"),
  terrainEntry.descriptor_sha256,
);
const terrain = JSON.parse(terrainBytes);
const terrainGameplay = structuredClone(terrain.gameplay);
const ground = terrainGameplay.surfaces.find((s) => s.id === contact.ground);
assert.ok(ground);
const hole = contact.hole;
const matches = ground.holes.filter((h) => JSON.stringify(h) === JSON.stringify(hole));
assert.equal(matches.length, 1, "Expected the reviewed road-platform terrain exclusion");
const holeIndex = ground.holes.indexOf(matches[0]);
if (ground.holeContours) ground.holeContours.splice(holeIndex, 1);
ground.holes = ground.holes.filter((h) => h !== matches[0]);
validateAssetGameplay(terrainGameplay, terrain);
edits.push({
  asset: terrainEntry.id,
  descriptorSha256: terrainEntry.descriptor_sha256,
  gameplay: terrainGameplay,
});
review.changes.push({
  asset: terrainEntry.id,
  surface: ground.id,
  removedHole: hole,
  replacement: gameplay.movementSolids,
});
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits, null, 2));
await fs.writeFile(`${output}/review.json`, JSON.stringify(review, null, 2));
console.log(JSON.stringify({ output, shifts }));
