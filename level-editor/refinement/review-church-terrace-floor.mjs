import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh, maskRecoveryTextures } from "../pipeline/src/mask-recovery-mesh.ts";
import { gltfToScene, sceneToGame } from "../shared/src/geometry.ts";

const [staged] = process.argv.slice(2);
assert.ok(staged);
const review = JSON.parse(await fs.readFile(`${staged}/review.json`, "utf8"));
assert.equal(review.asset, "leicester-lower-bailey-terrace");
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
const entry = index.find((e) => e.id === review.asset);
const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
assert.equal(hash(bytes), entry.descriptor_sha256);
const descriptor = JSON.parse(bytes);
const modelBytes = await fs.readFile(`library/3d-assets/${entry.model}`);
const model = await loadSceneModel("library", {
  id: entry.id,
  role: "objects",
  descriptor: `3d-assets/${entry.descriptor}`,
  descriptor_sha256: entry.descriptor_sha256,
  model: `3d-assets/${entry.model}`,
  model_sha256: hash(modelBytes),
  resources: descriptor.resources ?? [],
  ...(entry.model_scene ? { model_scene: entry.model_scene } : {}),
});
const triangles = maskRecoveryMesh(
  model,
  "building-123",
  (point) => sceneToGame({ kind: "oblique-orthographic", elevation_deg: 35 }, gltfToScene(point)),
  await maskRecoveryTextures(model),
);
const top = triangles.filter((triangle) =>
  triangle.every((p) => Math.abs(p[2] - review.height) < 0.05),
);
assert.ok(top.length);
function contains(p, [a, b, c]) {
  const det = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
  if (Math.abs(det) < 1e-8) return false;
  const u = ((b[1] - c[1]) * (p[0] - c[0]) + (c[0] - b[0]) * (p[1] - c[1])) / det;
  const v = ((c[1] - a[1]) * (p[0] - c[0]) + (a[0] - c[0]) * (p[1] - c[1])) / det;
  return u >= -1e-8 && v >= -1e-8 && u + v <= 1 + 1e-8;
}
function distance(p, a, b) {
  const dx = b[0] - a[0],
    dy = b[1] - a[1];
  const t = Math.max(
    0,
    Math.min(1, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / (dx * dx + dy * dy || 1)),
  );
  return Math.hypot(p[0] - a[0] - t * dx, p[1] - a[1] - t * dy);
}
const samples = [];
for (let along = 0; along <= 40; along++)
  for (let across = 0; across <= 4; across++) {
    const p = review.before[0].map((v, i) => v + ((review.before[1][i] - v) * along) / 40);
    const q = review.after[0].map((v, i) => v + ((review.after[1][i] - v) * along) / 40);
    const point = p.map((v, i) => v + ((q[i] - v) * across) / 4);
    const supported = top.some((triangle) => contains(point, triangle));
    const edgeDistance = supported
      ? 0
      : Math.min(
          ...top.flatMap((triangle) =>
            triangle.map((a, i) => distance(point, a, triangle[(i + 1) % 3])),
          ),
        );
    samples.push({ point, supported, edgeDistance });
  }
const result = {
  scope: "unpublished terrace seam mesh review",
  descriptorSha256: entry.descriptor_sha256,
  modelSha256: hash(modelBytes),
  topTriangles: top.length,
  samples: samples.length,
  supported: samples.filter((s) => s.supported).length,
  maximumUncoveredDistance: Math.max(...samples.map((s) => s.edgeDistance)),
};
await fs.writeFile(
  `${staged}/mesh-review.json`,
  JSON.stringify({ ...result, points: samples }, null, 2),
);
console.log(JSON.stringify(result));
