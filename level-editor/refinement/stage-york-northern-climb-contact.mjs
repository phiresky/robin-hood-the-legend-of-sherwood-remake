import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

// Explicit asset authoring only. The receiving wall must carry its contact
// geometry when placed independently; compilation never moves either asset.
const [stage] = process.argv.slice(2);
assert.ok(stage, "Provide reviewed climb seam edits");
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
assert.equal(edits.length, 1);
assert.equal(edits[0].asset, "york-east-riverside-northern-wall-stair");
const document = await readStoredMap("library/scenes/york.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
assert.equal(
  edits[0].descriptorSha256,
  index.find((entry) => entry.id === edits[0].asset).descriptor_sha256,
);
const origin = (id) => {
  const group = document.groups.find((group) => group.id === id);
  assert.ok(group && group.transform.rot_deg === 0);
  assert.ok(
    document.objects
      .filter((object) => object.group === id)
      .every((object) => Object.values(object.transform).every((value) => value === 0)),
  );
  return group.transform;
};
const id = "york-castle-south-curtain-wall";
const descriptor = assets.get(id);
const gameplay = structuredClone(descriptor.gameplay);
const climbOrigin = origin(edits[0].asset);
const receiverOrigin = origin(id);
const flight = edits[0].gameplay.surfaces.find((surface) => surface.id === "building-286-walk-0");
const plane = heightPlane(
  flight.polygon.map(([x, y], i) => [
    x + climbOrigin.dx - receiverOrigin.dx,
    y + climbOrigin.dy - receiverOrigin.dy,
    flight.height[i] + climbOrigin.dz - receiverOrigin.dz,
  ]),
);
const surface = gameplay.surfaces.find((surface) => surface.id === "building-768-walk-0");
assert.equal(surface.polygon.length, 17, "Receiving contour needs a fresh authoring review");
assert.equal(surface.projectionMaterials.footprint.length, 4);
const changes = [];
const contact = flight.polygon
  .filter((_, i) => Math.abs(flight.height[i] - 225.001) < 1e-6)
  .map(([x, y]) => [
    x + climbOrigin.dx - receiverOrigin.dx,
    y + climbOrigin.dy - receiverOrigin.dy,
  ]);
assert.equal(contact.length, 2);
const length = Math.hypot(contact[1][0] - contact[0][0], contact[1][1] - contact[0][1]);
const direction = contact[0].map((value, i) => (contact[1][i] - value) / length);
// Keep the contact local to this climb, with room for the actor at both ends.
// Distant wall corners retain their existing geometry and other connections.
const opening = contact.map((point, i) =>
  point.map((value, axis) => value + (i ? 8 : -8) * direction[axis]),
);
for (const [kind, points, edge, limit] of [
  ["walkable", surface.polygon, 7, 3],
  ["receiving", surface.projectionMaterials.footprint, 3, 0.5],
]) {
  const a = points[edge],
    b = points[(edge + 1) % points.length];
  const delta = [b[0] - a[0], b[1] - a[1]];
  const denominator = delta[0] ** 2 + delta[1] ** 2;
  const candidates = opening
    .map((point) => {
      const t = ((point[0] - a[0]) * delta[0] + (point[1] - a[1]) * delta[1]) / denominator;
      assert.ok(t > 0 && t < 1, "Contact must lie inside the selected wall edge");
      const distance = Math.hypot(point[0] - a[0] - t * delta[0], point[1] - a[1] - t * delta[1]);
      assert.ok(distance < limit, `${kind}: unreviewed movement ${distance}`);
      assert.ok(Math.abs(planeHeight(plane, point) - surface.height[0]) < 1e-6);
      return {
        t,
        distance,
        point: kind === "receiving" ? [...point, surface.height[0]] : [...point],
      };
    })
    .sort((a, b) => a.t - b.t);
  points.splice(edge + 1, 0, ...candidates.map((candidate) => candidate.point));
  if (kind === "walkable") surface.height.splice(edge + 1, 0, surface.height[0], surface.height[0]);
  changes.push({ kind, edge, inserted: candidates });
}
surface.preserveMovementPrecision = true;
surface.preserveMovementBoundary = true;
surface.navigationRegion = surface.id;
const openingId = "building-768-northern-climb-opening";
assert.ok(!gameplay.movementClearances?.some((entry) => entry.id === openingId));
const flightOpening = flight.polygon.map(([x, y]) => [
  x + climbOrigin.dx - receiverOrigin.dx,
  y + climbOrigin.dy - receiverOrigin.dy,
]);
gameplay.movementClearances ??= [];
gameplay.movementClearances.push({
  id: openingId,
  node: surface.node,
  polygon: flightOpening,
  height: flightOpening.map((point) => planeHeight(plane, point)),
  preserveMovementPrecision: true,
});
edits[0].gameplay.draft.issues.push(
  "Corrected northern climb flight has 908/920 visible mesh hits, maximum uncovered margin 0.117 units and height residual 0.023 units. Its sloped lower landing retains sampled gaps up to 0.241 units; rendered actor integration remains unverified.",
);
gameplay.draft.issues.push(
  "Northern climb receiving contact has 1021/1152 supported floor samples, maximum uncovered margin 0.413 units and height residual 0.062 units; rendered actor integration remains unverified.",
);
const crestClearance = edits[0].gameplay.movementClearances.find(
  (entry) => entry.id === "building-286-clearance-171-0",
);
assert.ok(crestClearance, "Expected an authored climb-crest opening");
const previousCrestClearance = structuredClone(crestClearance);
crestClearance.polygon = surface.polygon.map(([x, y]) => [
  x + receiverOrigin.dx - climbOrigin.dx,
  y + receiverOrigin.dy - climbOrigin.dy,
]);
crestClearance.height = surface.height.map((height) => height + receiverOrigin.dz - climbOrigin.dz);
crestClearance.preserveMovementPrecision = true;
changes.push({
  kind: "climb-crest-clearance",
  before: previousCrestClearance,
  after: crestClearance,
});
validateAssetGameplay(edits[0].gameplay, assets.get(edits[0].asset));
validateAssetGameplay(gameplay, descriptor);
edits.push({
  asset: id,
  descriptorSha256: index.find((entry) => entry.id === id).descriptor_sha256,
  gameplay,
});
// The neighbouring boundary wall already carries a climb opening. Its old
// sloped clearance must follow the corrected flight, in the wall's own frame.
const boundaryId = "york-east-riverside-northern-boundary-wall";
const boundary = assets.get(boundaryId);
const boundaryGameplay = structuredClone(boundary.gameplay);
const boundaryOrigin = origin(boundaryId);
const clearance = boundaryGameplay.movementClearances.find(
  (entry) => entry.id === "building-815-clearance-144-0",
);
assert.ok(clearance, "Expected an authored boundary-wall climb opening");
const previousClearance = structuredClone(clearance);
clearance.polygon = flight.polygon.map(([x, y]) => [
  x + climbOrigin.dx - boundaryOrigin.dx,
  y + climbOrigin.dy - boundaryOrigin.dy,
]);
clearance.height = flight.height.map((height) => height + climbOrigin.dz - boundaryOrigin.dz);
clearance.preserveMovementPrecision = true;
const upperClearance = boundaryGameplay.movementClearances.find(
  (entry) => entry.id === "building-815-clearance-171-0",
);
assert.ok(upperClearance, "Expected an authored upper-wall opening");
const previousUpperClearance = structuredClone(upperClearance);
upperClearance.polygon = surface.polygon.map(([x, y]) => [
  x + receiverOrigin.dx - boundaryOrigin.dx,
  y + receiverOrigin.dy - boundaryOrigin.dy,
]);
upperClearance.height = surface.height.map(
  (height) => height + receiverOrigin.dz - boundaryOrigin.dz,
);
upperClearance.preserveMovementPrecision = true;
boundaryGameplay.draft.issues.push(
  "Northern climb opening follows the corrected asset-local flight plane; rendered boundary-wall integration remains unverified.",
);
validateAssetGameplay(boundaryGameplay, boundary);
edits.push({
  asset: boundaryId,
  descriptorSha256: index.find((entry) => entry.id === boundaryId).descriptor_sha256,
  gameplay: boundaryGameplay,
});
changes.push({ kind: "boundary-climb-clearance", before: previousClearance, after: clearance });
changes.push({
  kind: "boundary-upper-clearance",
  before: previousUpperClearance,
  after: upperClearance,
});
const output = await fs.mkdtemp("work/map-compile/york-northern-climb-contact-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(`${output}/review.json`, JSON.stringify({ stage, changes }));
console.log(JSON.stringify({ output, changes }));
