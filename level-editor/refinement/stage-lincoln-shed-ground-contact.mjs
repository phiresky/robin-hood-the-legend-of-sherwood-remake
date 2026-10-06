import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh } from "../pipeline/src/mask-recovery-mesh.ts";
import { placeGameplaySurface } from "../shared/src/place-gameplay-surface.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { gameToScene } from "../shared/src/scene.ts";
import { gltfToScene, sceneToGame } from "../shared/src/geometry.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

// Author one reviewed contact in its owning terrain. Export never adjusts
// unrelated placements or invents a landing across a physical gap.
const document = await readStoredMap("library/scenes/lincoln.rhlos-map.json", "library");
const lowerApproachOffset = Number(process.argv[2] ?? 0);
assert.ok(
  Number.isFinite(lowerApproachOffset) && lowerApproachOffset >= 0 && lowerApproachOffset <= 3,
  "Lower approach offset must be between zero and three units",
);
const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
const transform = (id, node, point) => {
  const part = document.objects.find((object) => object.node === `asset:${id}:${node}`);
  assert.ok(part, `Missing placed frame ${id}/${node}`);
  const matrix = partMatrix(document.camera, document, part);
  const local = gameToScene(document.camera, ...point);
  return sceneToGame(
    document.camera,
    [0, 1, 2].map(
      (row) =>
        matrix[row] * local[0] +
        matrix[4 + row] * local[1] +
        matrix[8 + row] * local[2] +
        matrix[12 + row],
    ),
  );
};
const shed = assets.get("lincoln-courtyard-shed");
const lift = shed.gameplay.lifts.find((lift) => lift.id === "building-224-lift");
const floor = shed.gameplay.surfaces.find((surface) => surface.id === lift.surface);
const placed = placeGameplaySurface(floor, (node, point) => transform(shed.id, node, point));
const outside = transform(shed.id, lift.node, lift.doors[0].outside);
assert.ok(
  Math.hypot(...outside.map((v, i) => v - [1855, 1557.001, 220.001][i])) < 1e-6,
  "Reviewed shed placement changed",
);
const [a, b, c] = placed.worldPlane;
const z = outside[2];
const before = [
  [1840, 1332],
  [1860, 1327],
];
const after = before.map(([x, y]) => {
  const t = (a * x + b * (y + z) + c - z) / (a * a + b * b);
  assert.ok(Math.abs(t) * Math.hypot(a, b) < 2, "Contact exceeds reviewed shift bound");
  return [x - t * a, y - t * b];
});
const terrain = assets.get("lincoln-terrain");
const gameplay = structuredClone(terrain.gameplay);
const surface = gameplay.surfaces.find((surface) => surface.id === "ground-section-1-0");
const indices = before.map((point) =>
  surface.polygon.findIndex((p) => p.every((v, i) => v === point[i])),
);
assert.ok(indices.every((i) => i >= 0));
assert.equal((indices[0] + 1) % surface.polygon.length, indices[1]);
assert.ok(surface.height.every((height) => height === 0));
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
const plateauId = "lincoln-castle-hill-inner-bailey-plateau";
const entry = index.find((entry) => entry.id === plateauId);
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const modelBytes = await fs.readFile(`library/3d-assets/${entry.model}`);
const model = await loadSceneModel("library", {
  id: entry.id,
  role: "objects",
  descriptor: `3d-assets/${entry.descriptor}`,
  descriptor_sha256: entry.descriptor_sha256,
  model: `3d-assets/${entry.model}`,
  model_sha256: hash(modelBytes),
  resources: assets.get(plateauId).resources ?? [],
  ...(entry.model_scene ? { model_scene: entry.model_scene } : {}),
});
const triangles = maskRecoveryMesh(model, "building-062", (point) =>
  transform(plateauId, "building-062", sceneToGame(document.camera, gltfToScene(point))),
);
const meshHeight = (point, [a, b, c]) => {
  const det = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
  if (Math.abs(det) < 1e-8) return undefined;
  const u = ((b[1] - c[1]) * (point[0] - c[0]) + (c[0] - b[0]) * (point[1] - c[1])) / det;
  const v = ((c[1] - a[1]) * (point[0] - c[0]) + (a[0] - c[0]) * (point[1] - c[1])) / det;
  return u >= -1e-8 && v >= -1e-8 && u + v <= 1 + 1e-8
    ? u * a[2] + v * b[2] + (1 - u - v) * c[2]
    : undefined;
};
const samples = [];
for (let along = 0; along <= 40; along++)
  for (let across = 0; across <= 4; across++) {
    const p = before[0].map((v, i) => v + ((before[1][i] - v) * along) / 40);
    const q = after[0].map((v, i) => v + ((after[1][i] - v) * along) / 40);
    const projected = p.map((v, i) => v + ((q[i] - v) * across) / 4);
    const world = [projected[0], projected[1] + z, z];
    const hits = triangles
      .map((triangle) => meshHeight(world, triangle))
      .filter((height) => height !== undefined);
    samples.push({ projected, world, hits });
  }
// The mesh top is 0.0584 units below the authored receiving plane across this
// strip. This review bound changes no compiler/runtime connection tolerance.
const supported = samples.every((sample) =>
  sample.hits.some((height) => Math.abs(height - z) < 0.06),
);
const output = await fs.mkdtemp("work/map-compile/lincoln-shed-ground-contact-");
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify(
    {
      scope: "unpublished mesh-supported plateau contact",
      lowerApproachOffset,
      before,
      after,
      supported,
      modelSha256: hash(modelBytes),
      descriptorSha256: entry.descriptor_sha256,
      samples,
    },
    null,
    2,
  ),
);
assert.ok(supported, `Plateau mesh does not support contact: ${output}`);
indices.forEach((index, i) => (surface.polygon[index] = after[i]));
surface.preserveMovementPrecision = true;
validateAssetGameplay(gameplay, terrain);
const terrainEntry = index.find((entry) => entry.id === terrain.id);
const blockerAsset = assets.get("lincoln-castle-hill-keep-plateau");
const blockerGameplay = structuredClone(blockerAsset.gameplay);
const blocker = blockerGameplay.movementBlockers.find(
  (blocker) => blocker.id === "building-066-ground-blocker-4-0",
);
assert.ok(blocker);
const origin = transform(blockerAsset.id, blocker.node, [0, 0, 0]);
for (const [axis, point] of [
  [0, [1, 0, 0]],
  [1, [0, 1, 0]],
]) {
  const placed = transform(blockerAsset.id, blocker.node, point);
  assert.ok(
    placed.every((v, i) => Math.abs(v - origin[i] - (i === axis ? 1 : 0)) < 1e-7),
    "Reviewed blocker frame must be a translation",
  );
}
const blockerIndices = before.map((point) =>
  blocker.polygon.findIndex((local) => {
    const world = transform(blockerAsset.id, blocker.node, [...local, 0]);
    return Math.hypot(world[0] - point[0], world[1] - world[2] - point[1]) < 1e-6;
  }),
);
assert.ok(blockerIndices.every((i) => i >= 0));
assert.ok(
  (blockerIndices[0] + 1) % blocker.polygon.length === blockerIndices[1] ||
    (blockerIndices[1] + 1) % blocker.polygon.length === blockerIndices[0],
);
blockerIndices.forEach((position, i) => {
  blocker.polygon[position] = blocker.polygon[position].map(
    (v, axis) => v + after[i][axis] - before[i][axis],
  );
});
blocker.preserveMovementPrecision = true;
validateAssetGameplay(blockerGameplay, blockerAsset);
const edits = [
  {
    asset: terrain.id,
    descriptorSha256: terrainEntry.descriptor_sha256,
    gameplay,
  },
  {
    asset: blockerAsset.id,
    descriptorSha256: index.find((entry) => entry.id === blockerAsset.id).descriptor_sha256,
    gameplay: blockerGameplay,
  },
];
if (lowerApproachOffset) {
  const shedGameplay = structuredClone(shed.gameplay);
  const door = shedGameplay.lifts.find((item) => item.id === lift.id).doors[0];
  const length = Math.hypot(a, b);
  for (const point of [door.outside, door.middle, door.inside]) {
    point[0] += (lowerApproachOffset * b) / length;
    point[1] -= (lowerApproachOffset * a) / length;
  }
  shedGameplay.draft.issues = shedGameplay.draft.issues.map((issue) =>
    issue.replace(
      " The saved Lincoln ground contact still requires projected ladder navigation.",
      "",
    ),
  );
  validateAssetGameplay(shedGameplay, shed);
  edits.push({
    asset: shed.id,
    descriptorSha256: index.find((entry) => entry.id === shed.id).descriptor_sha256,
    gameplay: shedGameplay,
  });
}
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
console.log(JSON.stringify({ output, before, after, supported, samples: samples.length }));
