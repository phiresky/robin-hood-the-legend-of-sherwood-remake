import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh } from "../pipeline/src/mask-recovery-mesh.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { gameToScene } from "../shared/src/scene.ts";
import { gltfToScene, sceneToGame } from "../shared/src/geometry.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

// Review only the wall-owned edge overlapping the independent stair's landing.
const [stage, compiledFile] = process.argv.slice(2);
assert.ok(stage && compiledFile);
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
assert.ok(edits.some((e) => e.asset === "lincoln-north-hall-stair"));
const document = await readStoredMap("library/scenes/lincoln.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
const id = "lincoln-north-curtain-wall-west";
assert.ok(!edits.some((e) => e.asset === id));
const descriptor = assets.get(id),
  gameplay = structuredClone(descriptor.gameplay);
const surface = gameplay.surfaces.find((s) => s.id === "building-181-walk-0");
const part = document.objects.find((o) => o.node === `asset:${id}:${surface.node}`);
assert.ok(part);
const matrix = partMatrix(document.camera, document, part);
const transform = (point) => {
  const p = gameToScene(document.camera, ...point);
  return sceneToGame(
    document.camera,
    [0, 1, 2].map(
      (r) => matrix[r] * p[0] + matrix[4 + r] * p[1] + matrix[8 + r] * p[2] + matrix[12 + r],
    ),
  );
};
const origin = transform([0, 0, 0]);
for (const axis of [0, 1, 2]) {
  const p = [0, 0, 0];
  p[axis] = 1;
  assert.ok(transform(p).every((v, i) => Math.abs(v - origin[i] - (i === axis ? 1 : 0)) < 1e-7));
}
const compiled = JSON.parse(await fs.readFile(compiledFile, "utf8")).asset_geometry;
const lifts = compiled.lifts.filter((l) =>
  l.physical_navigation?.doors.some(
    (d) => Math.hypot(...d.outside.map((v, i) => v - [2027, 843.001, 370.001][i])) < 1e-5,
  ),
);
assert.equal(lifts.length, 1);
const floor = lifts[0].physical_navigation;
const [a, b, c] = floor.plane,
  z = 370.001,
  length = Math.hypot(a, b);
const seam = floor.boundary.filter(([x, y]) => Math.abs(a * x + b * y + c - z) < 1e-4);
assert.equal(seam.length, 2);
const side = ([x, y]) => (-b * x + a * y) / length;
const low = Math.min(...seam.map(side)),
  high = Math.max(...seam.map(side));
const edge = [
  [2308, 757.001],
  [1838, 923.001],
];
const start = surface.polygon.findIndex((_, i) =>
  edge.every(([x, y], j) => {
    const p = surface.polygon[(i + j) % surface.polygon.length];
    return Math.hypot(p[0] + origin[0] - x, p[1] + origin[1] - y) < 1e-6;
  }),
);
const indices = [start, (start + 1) % surface.polygon.length];
assert.ok(indices.every((i) => i >= 0));
assert.equal((indices[0] + 1) % surface.polygon.length, indices[1]);
const t = [low, high]
  .map((v) => (v - side(edge[0])) / (side(edge[1]) - side(edge[0])))
  .sort((a, b) => a - b);
assert.ok(t[0] > 0 && t[1] < 1);
const before = t.map((t) => edge[0].map((v, i) => v + (edge[1][i] - v) * t));
const after = before.map(([x, y]) => {
  const d = (a * x + b * y + c - z) / (a * a + b * b);
  assert.ok(Math.abs(d) * length < 2.5);
  return [x - d * a, y - d * b];
});
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
const entry = index.find((e) => e.id === id),
  hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const modelBytes = await fs.readFile(`library/3d-assets/${entry.model}`);
const model = await loadSceneModel("library", {
  id,
  role: "objects",
  descriptor: `3d-assets/${entry.descriptor}`,
  descriptor_sha256: entry.descriptor_sha256,
  model: `3d-assets/${entry.model}`,
  model_sha256: hash(modelBytes),
  resources: descriptor.resources ?? [],
  ...(entry.model_scene ? { model_scene: entry.model_scene } : {}),
});
const triangles = maskRecoveryMesh(model, surface.node, (p) =>
  transform(sceneToGame(document.camera, gltfToScene(p))),
);
function meshHeight(point, [a, b, c]) {
  const det = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
  if (Math.abs(det) < 1e-8) return undefined;
  const u = ((b[1] - c[1]) * (point[0] - c[0]) + (c[0] - b[0]) * (point[1] - c[1])) / det;
  const v = ((c[1] - a[1]) * (point[0] - c[0]) + (a[0] - c[0]) * (point[1] - c[1])) / det;
  return u >= -1e-8 && v >= -1e-8 && u + v <= 1 + 1e-8
    ? u * a[2] + v * b[2] + (1 - u - v) * c[2]
    : undefined;
}
const samples = [];
for (let along = 0; along <= 40; along++)
  for (let across = 0; across <= 4; across++) {
    const p = before[0].map((v, i) => v + ((before[1][i] - v) * along) / 40),
      q = after[0].map((v, i) => v + ((after[1][i] - v) * along) / 40);
    const point = p.map((v, i) => v + ((q[i] - v) * across) / 4);
    const hits = triangles.map((t) => meshHeight(point, t)).filter((h) => h !== undefined);
    samples.push({ point, hits });
  }
const supported = samples.every((s) => s.hits.some((h) => Math.abs(h - z) < 0.1));
const output = await fs.mkdtemp("work/map-compile/lincoln-north-hall-platform-");
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({ before, after, supported, samples, modelSha256: hash(modelBytes) }, null, 2),
);
assert.ok(supported, `Wall mesh does not support landing: ${output}`);
const points = [before[0], after[0], after[1], before[1]].map((p) =>
  p.map((v, i) => v - origin[i]),
);
assert.ok(surface.height.every((h) => Math.abs(h + origin[2] - z) < 1e-6));
surface.polygon.splice(indices[0] + 1, 0, ...points);
surface.height.splice(indices[0] + 1, 0, ...points.map(() => z - origin[2]));
surface.preserveMovementPrecision = true;
validateAssetGameplay(gameplay, descriptor);
edits.push({ asset: id, descriptorSha256: entry.descriptor_sha256, gameplay });
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits, null, 2));
console.log(
  JSON.stringify({ output, supportedSamples: samples.length, assets: edits.map((e) => e.asset) }),
);
