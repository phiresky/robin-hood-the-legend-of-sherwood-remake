import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { compileMap } from "../app/src/map-compile.ts";

// Explicit authoring candidate. Keep unpublished until both neighbouring stair
// routes, moved placements and visible mesh support have been reviewed.
const [terraceStage, stairStage, woodenStairStage] = process.argv.slice(2);
assert.ok(terraceStage && stairStage);
const edits = JSON.parse(await fs.readFile(`${terraceStage}/edits.json`, "utf8"));
const stairEdits = JSON.parse(await fs.readFile(`${stairStage}/edits.json`, "utf8"));
assert.equal(edits[0].asset, stairEdits[0].asset);
edits[0] = stairEdits[0];
if (woodenStairStage) {
  const woodenEdits = JSON.parse(await fs.readFile(`${woodenStairStage}/edits.json`, "utf8"));
  assert.equal(woodenEdits.length, 1);
  assert.equal(woodenEdits[0].asset, "york-riverbank-wooden-landing-steps");
  edits.push(woodenEdits[0]);
}
const document = await readStoredMap("library/scenes/york.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
const id = "york-east-riverside-wooden-walkway";
const descriptor = assets.get(id);
assert.ok(descriptor);
const gameplay = structuredClone(descriptor.gameplay);
assert.equal(gameplay.surfaces.length, 0);
assert.equal(gameplay.projectionReceivers.length, 1);
const receiver = gameplay.projectionReceivers[0];
const part = descriptor.parts.find((part) => part.node === receiver.volume);
assert.ok(part);
const points = part.obstacle_local_game.points;
assert.ok(points.every((p) => Math.abs(p.z_top - points[0].z_top) < 1e-7));
gameplay.projectionReceivers = [];
gameplay.movementSolids = [part.node];
gameplay.surfaces = [{
  id: `${part.node}-physical-walkway`,
  node: part.node,
  projectionVolume: part.node,
  polygon: points.map((p) => [p.x, p.y]),
  height: points.map((p) => p.z_top),
  holes: [],
  preserveMovementPrecision: true,
  preserveMovementBoundary: true,
  navigationRegion: `${part.node}-physical-walkway`,
}];
gameplay.draft.issues.push("Physical walkway floor candidate requires mesh, placement and rendered actor review before publication.");
validateAssetGameplay(gameplay, descriptor);
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
edits.push({ asset: id, descriptorSha256: index.find((entry) => entry.id === id).descriptor_sha256, gameplay });
for (const edit of edits) assets.get(edit.asset).gameplay = edit.gameplay;
const output = await fs.mkdtemp("work/map-compile/york-riverside-walkway-floor-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
const compiled = compileMap(document, [0, 0, ...document.size], assets, { bestEffort: true });
await fs.writeFile(`${output}/york.level.json`, JSON.stringify(compiled.descriptor));
await fs.writeFile(`${output}/diagnostics.json`, JSON.stringify({
  scope: "static-geometry-only-not-gameplay-parity",
  complete: true,
  results: [{ map: "york", file: "york.level.json", warnings: compiled.warnings }],
}));
console.log(JSON.stringify({ output, warnings: compiled.warnings.filter((warning) => /traversal omitted|wooden-landing-steps.*unavailable/.test(warning)) }));
