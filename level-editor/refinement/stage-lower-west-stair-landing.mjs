import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readStoredMap } from "../pipeline/src/stored-map.ts";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh, maskRecoveryTextures } from "../pipeline/src/mask-recovery-mesh.ts";
import { gltfToScene, sceneToGame } from "../shared/src/geometry.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

// Explicit authoring review: convert the placed seam into the neighbouring
// asset's local frame and verify the added receiving strip against its mesh.
const [staged, compiledFile, reviewOption] = process.argv.slice(2);
assert.ok(staged && compiledFile, "Provide staged stair/terrain edits and a Derby descriptor");
assert.ok(reviewOption === undefined || reviewOption === "--reviewed-mesh-edge=0.25");
// Explicit mesh review permits only this asset's sub-unit edge discrepancy.
// This tolerance never enters the compiler or runtime landing attachment rules.
const meshEdgeTolerance = reviewOption ? 0.25 : 0;
const edits = JSON.parse(await fs.readFile(`${staged}/edits.json`, "utf8"));
assert.ok(edits.some((edit) => edit.asset === "derby-lower-west-access-stair"));
const bytes = await fs.readFile(compiledFile);
const compiled = JSON.parse(bytes).asset_geometry;
const lifts = compiled.lifts.filter((lift) =>
  lift.physical_navigation?.doors.some(
    (door) =>
      Math.hypot(door.outside[0] - 362, door.outside[1] - 1685.001, door.outside[2] - 150.001) <
      1e-5,
  ),
);
assert.equal(lifts.length, 1);
const asset = "derby-lower-west-curtain";
const scene = await readStoredMap("library/scenes/derby.rhlos-map.json", "library");
const group = scene.groups.find((group) => group.id === asset);
assert.ok(group && group.transform.rot_deg === 0, "Reviewed placement frame changed");
const parts = scene.objects.filter((part) => part.group === group.id);
assert.ok(parts.every((part) => Object.values(part.transform).every((value) => value === 0)));
const { dx, dy, dz } = group.transform;
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
const entry = index.find((entry) => entry.id === asset);
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const descriptorBytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
assert.equal(hash(descriptorBytes), entry.descriptor_sha256);
const descriptor = JSON.parse(descriptorBytes);
const gameplay = structuredClone(descriptor.gameplay);
const surface = gameplay.surfaces.find((surface) => surface.id === "building-045-walk-0");
const height = surface.height[0];
assert.ok(surface.height.every((value) => value === height));
assert.ok(Math.abs(height + dz - 150.001) < 1e-6);
const before = [surface.polygon[14], surface.polygon[15]];
assert.ok(
  before.every(
    (point, i) =>
      Math.hypot(point[0] + dx - [387, 358][i], point[1] + dy - [1692.001, 1698.001][i]) < 1e-5,
  ),
  "Reviewed wall edge changed",
);
const [a, b, c] = lifts[0].physical_navigation.plane;
const localC = c + a * dx + b * dy - dz;
const after = before.map(([x, y]) => {
  const distance = (a * x + b * y + localC - height) / (a * a + b * b);
  assert.ok(Math.abs(distance) * Math.hypot(a, b) < 2.5, "Landing needs broader review");
  return [x - distance * a, y - distance * b];
});
const modelBytes = await fs.readFile(`library/3d-assets/${entry.model}`);
const model = await loadSceneModel("library", {
  id: asset,
  role: "objects",
  descriptor: `3d-assets/${entry.descriptor}`,
  descriptor_sha256: entry.descriptor_sha256,
  model: `3d-assets/${entry.model}`,
  model_sha256: hash(modelBytes),
  resources: descriptor.resources ?? [],
  ...(entry.model_scene ? { model_scene: entry.model_scene } : {}),
});
const textures = await maskRecoveryTextures(model);
const triangles = maskRecoveryMesh(
  model,
  surface.node,
  (point) => sceneToGame(scene.camera, gltfToScene(point)),
  textures,
);
const stairAsset = "derby-lower-west-access-stair";
const stairEntry = index.find((entry) => entry.id === stairAsset);
const stairGroup = scene.groups.find((group) => group.id === stairAsset);
assert.ok(stairGroup && stairGroup.transform.rot_deg === 0);
const stairBytes = await fs.readFile(`library/3d-assets/${stairEntry.descriptor}`);
assert.equal(hash(stairBytes), stairEntry.descriptor_sha256);
const stairDescriptor = JSON.parse(stairBytes);
const stairModelBytes = await fs.readFile(`library/3d-assets/${stairEntry.model}`);
const stairModel = await loadSceneModel("library", {
  id: stairAsset,
  role: "objects",
  descriptor: `3d-assets/${stairEntry.descriptor}`,
  descriptor_sha256: stairEntry.descriptor_sha256,
  model: `3d-assets/${stairEntry.model}`,
  model_sha256: hash(stairModelBytes),
  resources: stairDescriptor.resources ?? [],
  ...(stairEntry.model_scene ? { model_scene: stairEntry.model_scene } : {}),
});
const stairTriangles = maskRecoveryMesh(
  stairModel,
  "building-038",
  (point) => {
    const local = sceneToGame(scene.camera, gltfToScene(point));
    return local.map(
      (value, axis) =>
        value +
        [stairGroup.transform.dx - dx, stairGroup.transform.dy - dy, stairGroup.transform.dz - dz][
          axis
        ],
    );
  },
  await maskRecoveryTextures(stairModel),
);
function meshHeight(point, triangle) {
  const [a, b, c] = triangle;
  const det = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
  if (Math.abs(det) < 1e-8) return undefined;
  const u = ((b[1] - c[1]) * (point[0] - c[0]) + (c[0] - b[0]) * (point[1] - c[1])) / det;
  const v = ((c[1] - a[1]) * (point[0] - c[0]) + (a[0] - c[0]) * (point[1] - c[1])) / det;
  return u >= -1e-8 && v >= -1e-8 && u + v <= 1 + 1e-8
    ? u * a[2] + v * b[2] + (1 - u - v) * c[2]
    : undefined;
}
const samples = [];
const receivingTriangles = triangles.filter((triangle) =>
  triangle.every((p) => Math.abs(p[2] - height) < 0.1),
);
function edgeDistance(point, a, b) {
  const dx = b[0] - a[0],
    dy = b[1] - a[1];
  const t = Math.max(
    0,
    Math.min(1, ((point[0] - a[0]) * dx + (point[1] - a[1]) * dy) / (dx * dx + dy * dy || 1)),
  );
  return Math.hypot(point[0] - a[0] - t * dx, point[1] - a[1] - t * dy);
}
for (let along = 0; along <= 40; along++)
  for (let across = 0; across <= 4; across++) {
    const old = before[0].map((value, axis) => value + ((before[1][axis] - value) * along) / 40);
    const next = after[0].map((value, axis) => value + ((after[1][axis] - value) * along) / 40);
    const point = old.map((value, axis) => value + ((next[axis] - value) * across) / 4);
    const hits = triangles
      .map((triangle) => meshHeight(point, triangle))
      .filter((height) => height !== undefined);
    const receivingEdgeDistance = Math.min(
      ...receivingTriangles.flatMap((triangle) =>
        triangle.map((a, i) => edgeDistance(point, a, triangle[(i + 1) % 3])),
      ),
    );
    const stairHits = stairTriangles
      .map((triangle) => meshHeight(point, triangle))
      .filter((height) => height !== undefined);
    samples.push({ point, hits, receivingEdgeDistance, stairHits });
  }
const output = await fs.mkdtemp("work/map-compile/lower-west-stair-landing-");
const supported = samples.every((sample) => sample.hits.some((z) => Math.abs(z - height) < 0.1));
const accepted = samples.every(
  (sample) =>
    sample.hits.some((z) => Math.abs(z - height) < 0.1) ||
    sample.receivingEdgeDistance <= meshEdgeTolerance,
);
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify(
    {
      scope: "unpublished mesh-reviewed wall landing",
      compiledFile,
      compiledSha256: hash(bytes),
      descriptorSha256: entry.descriptor_sha256,
      modelSha256: hash(modelBytes),
      stairDescriptorSha256: stairEntry.descriptor_sha256,
      stairModelSha256: hash(stairModelBytes),
      transform: group.transform,
      height,
      before,
      after,
      supported,
      accepted,
      meshEdgeTolerance,
      samples,
    },
    null,
    2,
  ),
);
const bounds = [0, 1].map((axis) => [
  Math.min(...before.concat(after).map((point) => point[axis])) - 3,
  Math.max(...before.concat(after).map((point) => point[axis])) + 3,
]);
const svgPoints = (points) => points.map((p) => `${p[0]},${p[1]}`).join(" ");
const near = (triangle) =>
  triangle.some(
    (p) =>
      p[0] >= bounds[0][0] && p[0] <= bounds[0][1] && p[1] >= bounds[1][0] && p[1] <= bounds[1][1],
  );
await fs.writeFile(
  `${output}/contact.svg`,
  `<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="500" viewBox="${bounds[0][0]} ${bounds[1][0]} ${bounds[0][1] - bounds[0][0]} ${bounds[1][1] - bounds[1][0]}">
<rect x="${bounds[0][0]}" y="${bounds[1][0]}" width="100%" height="100%" fill="white"/>
${receivingTriangles
  .filter(near)
  .map(
    (t) =>
      `<polygon points="${svgPoints(t)}" fill="#88bbff" fill-opacity=".5" stroke="#4477aa" stroke-width=".03"/>`,
  )
  .join("")}
${stairTriangles
  .filter((t) => near(t) && t.every((p) => p[2] > height - 10))
  .map(
    (t) =>
      `<polygon points="${svgPoints(t)}" fill="#eeaa66" fill-opacity=".4" stroke="#995511" stroke-width=".03"/>`,
  )
  .join("")}
<polyline points="${svgPoints(before)}" fill="none" stroke="green" stroke-width=".08"/>
<polyline points="${svgPoints(after)}" fill="none" stroke="red" stroke-width=".08"/>
</svg>`,
);
assert.ok(accepted, `Wall mesh does not support the contact; see ${output}`);
[14, 15].forEach((index, i) => (surface.polygon[index] = after[i]));
surface.preserveMovementPrecision = true;
validateAssetGameplay(gameplay, descriptor);
edits.push({ asset, descriptorSha256: entry.descriptor_sha256, gameplay });
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
console.log(
  JSON.stringify({
    output,
    supported,
    accepted,
    meshEdgeTolerance,
    samples: samples.length,
    before,
    after,
  }),
);
