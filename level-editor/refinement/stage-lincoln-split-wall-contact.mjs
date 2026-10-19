// Author a shared contact in the two owning frames, using the measured cap edge.
// This candidate does not change models, unrelated collision or runtime routing.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh } from "../pipeline/src/mask-recovery-mesh.ts";
import { sceneToGame, gltfToScene } from "../shared/src/geometry.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const [stage] = process.argv.slice(2);
assert.ok(stage, "Supply the calibrated and mesh-clipped wall stage");
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`));
const scene = JSON.parse(await fs.readFile("library/scenes/lincoln.rhlos-map.json"));
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const loaded = [];
for (const id of ["lincoln-east-slate-tower", "lincoln-east-curtain-wall-south"]) {
  const root = `3d-assets/lincoln/${id}`;
  const bytes = await fs.readFile(`library/${root}/asset.json`);
  const descriptor = JSON.parse(bytes);
  const edit = edits.find((edit) => edit.asset === id);
  assert.ok(edit);
  assert.equal(hash(bytes), edit.descriptorSha256);
  const modelSha256 = hash(await fs.readFile(`library/${root}/${descriptor.model}`));
  assert.equal(modelSha256, edit.modelSha256);
  const placement = scene.placements.find((placement) => placement.id === id);
  assert.ok(placement && placement.assets.length === 1 && placement.assets[0] === id);
  assert.equal(placement.transform.rot_deg, 0);
  assert.equal(placement.transform.dz, 0);
  assert.equal(edit.gameplay.surfaces.length, 1);
  const floor = edit.gameplay.surfaces[0];
  assert.ok(floor.height.every((height) => height === 344.001));
  loaded.push({ id, root, descriptor, edit, placement, floor, modelSha256 });
}
const [north, south] = loaded;
const model = await loadSceneModel("library", {
  id: north.id,
  role: "objects",
  descriptor: `${north.root}/asset.json`,
  descriptor_sha256: north.edit.descriptorSha256,
  model: `${north.root}/${north.descriptor.model}`,
  model_sha256: north.modelSha256,
  model_scene: north.descriptor.model_scene,
  resources: north.descriptor.resources ?? [],
});
const cap = maskRecoveryMesh(model, north.floor.node, (point) =>
  sceneToGame(scene.camera, gltfToScene(point)),
).filter((triangle) => triangle.every((point) => Math.abs(point[2] - 344) < 0.01));
// Reviewed corner seeds select actual mesh vertices; measured values are used
// for the contact rather than rounded catalog coordinates.
const corners = [
  [-22, 14.408],
  [2, 21.208],
].map((seed) => {
  const hits = cap
    .flat()
    .filter((point) => Math.hypot(point[0] - seed[0], point[1] - seed[1]) < 0.01);
  assert.ok(hits.length, "Reviewed cap corner is no longer present");
  assert.ok(hits.every((point) => Math.hypot(...point.map((v, i) => v - hits[0][i])) < 1e-6));
  return hits[0].slice(0, 2);
});
const project = (point, [a, b]) => {
  const dx = b[0] - a[0],
    dy = b[1] - a[1];
  const t = ((point[0] - a[0]) * dx + (point[1] - a[1]) * dy) / (dx * dx + dy * dy);
  return [a[0] + t * dx, a[1] + t * dy];
};
const before = loaded.map(({ id, floor }) => ({ asset: id, floor: structuredClone(floor) }));
assert.equal(north.floor.polygon.length, 20);
assert.equal(south.floor.polygon.length, 9);
const northIndices = [15, 16];
for (const index of northIndices) {
  const p = north.floor.polygon[index],
    q = project(p, corners);
  assert.ok(Math.hypot(q[0] - p[0], q[1] - p[1]) < 6, "North extension exceeds reviewed strip");
  north.floor.polygon[index] = q;
}
const delta = [
  north.placement.transform.dx - south.placement.transform.dx,
  north.placement.transform.dy - south.placement.transform.dy,
];
const southLine = corners.map((point) => point.map((value, axis) => value + delta[axis]));
for (const index of [0, 1, 8]) {
  const p = south.floor.polygon[index],
    q = project(p, southLine);
  assert.ok(
    Math.hypot(q[0] - p[0], q[1] - p[1]) < 1,
    "South contact exceeds reviewed cap alignment",
  );
  south.floor.polygon[index] = q;
}
// One authored socket must lie on one edge. The middle sample is now collinear
// with the contact, so remove it without changing the resulting floor boundary.
south.floor.polygon.splice(0, 1);
south.floor.height.splice(0, 1);
const northEdge = northIndices.map((index) => [...north.floor.polygon[index], 344.001]);
const southEdge = northEdge.toReversed().map(([x, y, z]) => [x + delta[0], y + delta[1], z]);
for (const [asset, edge] of [
  [north, northEdge],
  [south, southEdge],
]) {
  assert.equal(asset.floor.navigationJoins, undefined);
  asset.floor.navigationRegion = asset.floor.id;
  asset.floor.navigationJoins = [edge];
  asset.floor.preserveMovementPrecision = true;
  validateAssetGameplay(asset.edit.gameplay, asset.descriptor);
}
const output = await fs.mkdtemp("work/needs-review/map-compile/split-wall-contact-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({
    stage,
    published: false,
    corners,
    delta,
    before,
    after: loaded.map(({ id, floor, modelSha256 }) => ({ asset: id, floor, modelSha256 })),
    scope:
      "asset-local shared cap contact; requires mesh and independent native placement verification",
  }),
);
console.log(output);
