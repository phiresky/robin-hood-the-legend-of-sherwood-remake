import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { compileMap } from "../app/src/map-compile.ts";

// Author the independent terrace contact; no scene-specific correction runs
// during export. Publication requires both stair and neighbouring route checks.
const [stage] = process.argv.slice(2);
assert.ok(stage);
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
assert.equal(edits.length, 1);
assert.equal(edits[0].asset, "york-riverbank-stone-landing-steps");
const document = await readStoredMap("library/scenes/york.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
const stair = edits[0].gameplay;
const lift = stair.lifts[0];
const floor = stair.surfaces.find((surface) => surface.id === lift.surface);
const placement = document.groups.find((group) => group.id === edits[0].asset).transform;
assert.equal(placement.rot_deg, 0);
const plane = heightPlane(
  floor.polygon.map(([x, y], i) => [
    x + placement.dx,
    y + placement.dy,
    floor.height[i] + placement.dz,
  ]),
);
const id = "york-southeast-riverside-raised-terrace";
const descriptor = assets.get(id);
const gameplay = structuredClone(descriptor.gameplay);
const terracePlacement = document.groups.find((group) => group.id === id).transform;
assert.equal(terracePlacement.rot_deg, 0);
const surface = gameplay.surfaces.find((surface) => surface.id === "building-093-physical-walkway");
const volume = gameplay.volumes.find((volume) => volume.id === surface.projectionVolume);
const changes = [];
for (const corner of [3, 4]) {
  const point = volume.shape.points[corner];
  const before = structuredClone(point);
  const world = [point.x + terracePlacement.dx, point.y + terracePlacement.dy];
  const t =
    (point.z_top + terracePlacement.dz - planeHeight(plane, world)) /
    (plane[0] ** 2 + plane[1] ** 2);
  const distance = Math.abs(t) * Math.hypot(plane[0], plane[1]);
  assert.ok(distance < 1, `Unreviewed terrace correction ${distance}`);
  point.x += t * plane[0];
  point.y += t * plane[1];
  changes.push({ corner, before, after: structuredClone(point), distance });
}
surface.polygon = volume.shape.points.map((point) => [point.x, point.y]);
surface.height = volume.shape.points.map((point) => point.z_top);
gameplay.draft.issues.push(
  "Stone stair terrace contact candidate requires mesh and independent placement review before publication.",
);
validateAssetGameplay(gameplay, descriptor);
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
edits.push({
  asset: id,
  descriptorSha256: index.find((entry) => entry.id === id).descriptor_sha256,
  gameplay,
});
const output = await fs.mkdtemp("work/map-compile/york-stone-stair-contact-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(`${output}/review.json`, JSON.stringify({ stage, changes }));
for (const edit of edits) assets.get(edit.asset).gameplay = edit.gameplay;
const compiled = compileMap(document, [0, 0, ...document.size], assets, { bestEffort: true });
await fs.writeFile(`${output}/york.level.json`, JSON.stringify(compiled.descriptor));
await fs.writeFile(
  `${output}/diagnostics.json`,
  JSON.stringify({
    scope: "static-geometry-only-not-gameplay-parity",
    complete: true,
    results: [{ map: "york", file: "york.level.json", warnings: compiled.warnings }],
  }),
);
console.log(JSON.stringify({ output, changes }));
