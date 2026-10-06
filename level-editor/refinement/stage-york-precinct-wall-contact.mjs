import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

// Explicit authoring candidate. Compilation never moves a receiver to meet a
// placed climb: these two assets retain their own independently placed geometry.
const [stage] = process.argv.slice(2);
assert.ok(stage, "Provide reviewed climb seam edits");
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
assert.equal(edits.length, 1);
assert.equal(edits[0].asset, "york-precinct-southwest-wall-ramp");
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
const id = "york-cathedral-precinct-raised-terrain";
const descriptor = assets.get(id);
const gameplay = structuredClone(descriptor.gameplay);
const rampOrigin = origin(edits[0].asset);
const receiverOrigin = origin(id);
const flight = edits[0].gameplay.surfaces.find((surface) => surface.id === "building-109-walk-0");
const plane = heightPlane(
  flight.polygon.map(([x, y], i) => [
    x + rampOrigin.dx - receiverOrigin.dx,
    y + rampOrigin.dy - receiverOrigin.dy,
    flight.height[i] + rampOrigin.dz - receiverOrigin.dz,
  ]),
);
const surface = gameplay.surfaces.find((surface) => surface.id === "building-087-walk-0");
const changes = [];
for (const [vertex, expected] of [
  [78, [-129, 124.05188495939194]],
  [79, [-174, 114.05188495939197]],
]) {
  const point = surface.polygon[vertex];
  assert.ok(Math.hypot(point[0] - expected[0], point[1] - expected[1]) < 1e-6);
  const before = [...point];
  const height = Array.isArray(surface.height) ? surface.height[vertex] : surface.height;
  const t = (height - planeHeight(plane, point)) / (plane[0] ** 2 + plane[1] ** 2);
  const distance = Math.abs(t) * Math.hypot(plane[0], plane[1]);
  assert.ok(distance < 3, `Contact exceeds reviewed authoring bound: ${distance}`);
  point[0] += t * plane[0];
  point[1] += t * plane[1];
  changes.push({ surface: surface.id, vertex, before, after: [...point], distance });
}
surface.preserveMovementPrecision = true;
surface.preserveMovementBoundary = true;
surface.navigationRegion = surface.id;
// The climb's separate crest reaches the upper landing height. Its own exit
// must remain clear there, independently of any surrounding buildings.
const ramp = edits[0].gameplay;
assert.ok(!ramp.movementClearances?.some((entry) => entry.id === "building-877-climb-exit"));
ramp.movementClearances ??= [];
const crest = assets.get(edits[0].asset).parts.find((part) => part.node === "building-877");
assert.ok(crest?.obstacle_local_game?.points?.length >= 3);
const crestOpening = crest.obstacle_local_game.points.map((point) => [point.x, point.y]);
ramp.movementClearances.push({
  id: "building-877-climb-exit",
  node: crest.node,
  polygon: crestOpening,
  height: crestOpening.map(() => ramp.lifts[0].doors[1].outside[2]),
  preserveMovementPrecision: true,
});
for (const vertex of [15, 16]) {
  const point = surface.projectionMaterials.footprint[vertex];
  const before = [...point];
  const t = (point[2] - planeHeight(plane, point)) / (plane[0] ** 2 + plane[1] ** 2);
  const distance = Math.abs(t) * Math.hypot(plane[0], plane[1]);
  assert.ok(distance < 0.5, "Receiving footprint contact exceeds authoring bound");
  point[0] += t * plane[0];
  point[1] += t * plane[1];
  changes.push({ receiver: surface.id, vertex, before, after: [...point], distance });
}
edits[0].gameplay.draft.issues.push(
  "Corrected climb floor has 808/811 visible mesh hits, uncovered margins up to 0.012 units and height residuals up to 0.268 units; rendered actor integration remains unverified.",
);
gameplay.draft.issues.push(
  "Reviewed southwest climb receiving contact retains incomplete mesh coverage: 4732/4944 floor samples supported, maximum gap 0.282 units and height residual 0.062 units; rendered integration remains unverified.",
);
validateAssetGameplay(gameplay, descriptor);
validateAssetGameplay(ramp, assets.get(edits[0].asset));
edits.push({
  asset: id,
  descriptorSha256: index.find((entry) => entry.id === id).descriptor_sha256,
  gameplay,
});
const output = await fs.mkdtemp("work/map-compile/york-precinct-wall-contact-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(`${output}/review.json`, JSON.stringify({ stage, changes }));
console.log(JSON.stringify({ output, changes }));
