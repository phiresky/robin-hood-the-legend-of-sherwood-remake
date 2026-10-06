import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap } from "../pipeline/src/stored-map.ts";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh } from "../pipeline/src/mask-recovery-mesh.ts";
import { gltfToScene, sceneToGame } from "../shared/src/geometry.ts";

// Camera-facing ground artwork can establish coverage, not physical support.
const [stage, map] = process.argv.slice(2);
assert.ok(stage && map);
const review = JSON.parse(await fs.readFile(`${stage}/review.json`, "utf8"));
const changes = review.changes.filter((change) => change.asset === `${map}-terrain`);
assert.equal(changes.length, 2, "Review one receiving edge at a time");
const document = await readStoredMap(`library/scenes/${map}.rhlos-map.json`, "library");
const source = document.sceneAssets.find((asset) => asset.id === `${map}-terrain`);
assert.equal(source.role, "ground");
const model = await loadSceneModel("library", source);
const triangles = maskRecoveryMesh(model, "ground", (point) => {
  const [x, y, z] = sceneToGame(document.camera, gltfToScene(point));
  assert.ok(Math.abs(y) < 0.001, "Review changed backdrop orientation");
  return [x, y - z, 0];
});
function covers([x, y], [a, b, c]) {
  const determinant = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
  if (Math.abs(determinant) < 1e-8) return false;
  const u = ((b[1] - c[1]) * (x - c[0]) + (c[0] - b[0]) * (y - c[1])) / determinant;
  const v = ((c[1] - a[1]) * (x - c[0]) + (a[0] - c[0]) * (y - c[1])) / determinant;
  return u >= -1e-8 && v >= -1e-8 && u + v <= 1 + 1e-8;
}
const samples = [];
for (let along = 0; along <= 40; along++)
  for (let across = 0; across <= 4; across++) {
    const point = [0, 1].map((axis) => {
      const before =
        changes[0].before[axis] +
        ((changes[1].before[axis] - changes[0].before[axis]) * along) / 40;
      const after =
        changes[0].after[axis] + ((changes[1].after[axis] - changes[0].after[axis]) * along) / 40;
      return before + ((after - before) * across) / 4;
    });
    samples.push({ point, covered: triangles.some((triangle) => covers(point, triangle)) });
  }
const report = {
  scope: "projected-backdrop-coverage-not-physical-support",
  modelSha256: source.model_sha256,
  samples,
};
await fs.writeFile(`${stage}/ground-coverage.json`, JSON.stringify(report));
assert.ok(
  samples.every((sample) => sample.covered),
  "Contact leaves the ground artwork",
);
console.log(JSON.stringify({ stage, covered: samples.length }));
