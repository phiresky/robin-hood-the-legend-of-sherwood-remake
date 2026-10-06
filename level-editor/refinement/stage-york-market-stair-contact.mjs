import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh } from "../pipeline/src/mask-recovery-mesh.ts";
import { sceneToGame, gltfToScene } from "../shared/src/geometry.ts";
import { compileMap } from "../app/src/map-compile.ts";

const [stage] = process.argv.slice(2);
assert.ok(stage);
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
assert.equal(edits.length, 1);
assert.equal(edits[0].asset, "york-market-southwest-connecting-stairs");
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
const landingHeight = lift.doors[0].outside[2] + placement.dz;
const id = "york-west-town-raised-terrain";
const descriptor = assets.get(id);
const gameplay = structuredClone(descriptor.gameplay);
const receiverPlacement = document.groups.find((group) => group.id === id).transform;
assert.equal(receiverPlacement.rot_deg, 0);
assert.equal(receiverPlacement.dz, 0);
const blocker = gameplay.movementBlockers.find(
  (blocker) => blocker.id === "building-086-ground-blocker-368-8-0",
);
assert.ok(blocker.height.every((z) => z === 0));
const before = [25, 26].map((i) => [...blocker.polygon[i]]);
const changes = [];
for (const corner of [25, 26]) {
  const point = blocker.polygon[corner];
  const world = [point[0] + receiverPlacement.dx, point[1] + receiverPlacement.dy + landingHeight];
  const t = (landingHeight - planeHeight(plane, world)) / (plane[0] ** 2 + plane[1] ** 2);
  const distance = Math.abs(t) * Math.hypot(plane[0], plane[1]);
  assert.ok(distance < 1, `Unexpected projected contact correction ${distance}`);
  point[0] += t * plane[0];
  point[1] += t * plane[1];
  changes.push({ corner, distance });
}
blocker.preserveMovementPrecision = true;
const after = [25, 26].map((i) => blocker.polygon[i]);
// These contours describe collision on the raised receiver, while its route
// graph shares a lower navigation plane. Restore their physical local frame
// before rotation; retain the separate graph height explicitly.
for (const contour of gameplay.movementBlockers) {
  assert.ok(contour.height.every((z) => z === 0));
  contour.polygon = contour.polygon.map(([x, y]) => [x, y + landingHeight]);
  contour.holes = (contour.holes ?? []).map((hole) => hole.map(([x, y]) => [x, y + landingHeight]));
  contour.height = contour.height.map(() => landingHeight);
  contour.navigationHeight = 0;
}
for (const receiver of gameplay.projectionReceivers) {
  assert.equal(receiver.anchor[2], 0);
  receiver.anchor[1] += landingHeight;
  receiver.anchor[2] = landingHeight;
  receiver.navigationHeight = 0;
}
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
const entry = index.find((entry) => entry.id === id);
const modelSha256 = createHash("sha256")
  .update(await fs.readFile(`library/3d-assets/${entry.model}`))
  .digest("hex");
const model = await loadSceneModel("library", {
  id,
  role: "objects",
  model: `3d-assets/${entry.model}`,
  model_sha256: modelSha256,
  descriptor: `3d-assets/${entry.descriptor}`,
  descriptor_sha256: entry.descriptor_sha256,
  resources: descriptor.resources ?? [],
});
const triangles = maskRecoveryMesh(model, blocker.node, (p) =>
  sceneToGame(document.camera, gltfToScene(p)),
);
function meshHeight(p, [a, b, c]) {
  const det = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
  if (Math.abs(det) < 1e-8) return undefined;
  const u = ((b[1] - c[1]) * (p[0] - c[0]) + (c[0] - b[0]) * (p[1] - c[1])) / det;
  const v = ((c[1] - a[1]) * (p[0] - c[0]) + (a[0] - c[0]) * (p[1] - c[1])) / det;
  return u >= -1e-8 && v >= -1e-8 && u + v <= 1 + 1e-8
    ? u * a[2] + v * b[2] + (1 - u - v) * c[2]
    : undefined;
}
const samples = [];
for (let i = 0; i <= 40; i++)
  for (let j = 0; j <= 4; j++) {
    const point = [0, 1].map((axis) => {
      const p = before[0][axis] + ((before[1][axis] - before[0][axis]) * i) / 40;
      const q = after[0][axis] + ((after[1][axis] - after[0][axis]) * i) / 40;
      return p + ((q - p) * j) / 4 + (axis === 1 ? landingHeight : 0);
    });
    const hits = triangles
      .map((triangle) => meshHeight(point, triangle))
      .filter((z) => z !== undefined);
    samples.push({ point, supported: hits.some((z) => Math.abs(z - landingHeight) < 0.1) });
  }
const output = await fs.mkdtemp("work/map-compile/york-market-stair-contact-");
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({ stage, modelSha256, before, after, changes, samples }),
);
assert.ok(
  samples.every((sample) => sample.supported),
  `Receiving mesh does not support corrected contact: ${output}`,
);
gameplay.draft.issues.push(
  "Market stair contact has complete sampled receiving mesh support; native moved traversal and rendered actor review remain required.",
);
validateAssetGameplay(gameplay, descriptor);
edits.push({ asset: id, descriptorSha256: entry.descriptor_sha256, gameplay });
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
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
console.log(JSON.stringify({ output, changes, samples: samples.length }));
