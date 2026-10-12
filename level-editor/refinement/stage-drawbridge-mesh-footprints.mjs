// Keep the staged deck navigation within its visible near-horizontal mesh.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh } from "../pipeline/src/mask-recovery-mesh.ts";
import { sceneToGame, gltfToScene } from "../shared/src/geometry.ts";
import { fixedPolygonBoolean } from "../shared/src/fixed-polygon-boolean.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const [stage] = process.argv.slice(2);
assert.ok(stage, "Supply staged drawbridge and plateau edits");
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`));
const edit = edits.find((entry) => entry.asset === "lincoln-south-gate-drawbridge");
assert.ok(edit);
const root = "3d-assets/lincoln/lincoln-south-gate-drawbridge";
const bytes = await fs.readFile(`library/${root}/asset.json`);
assert.equal(createHash("sha256").update(bytes).digest("hex"), edit.descriptorSha256);
const descriptor = JSON.parse(bytes);
const model = await loadSceneModel("library", {
  id: edit.asset,
  role: "objects",
  model: `${root}/model.glb`,
  model_sha256: edit.modelSha256,
  descriptor: `${root}/asset.json`,
  descriptor_sha256: edit.descriptorSha256,
  resources: descriptor.resources ?? [],
});
const camera = { kind: "oblique-orthographic", elevation_deg: 35 };
const changes = [];
for (const surface of edit.gameplay.surfaces) {
  const height = surface.height[0];
  assert.ok(surface.height.every((z) => z === height));
  const triangles = maskRecoveryMesh(model, surface.node, (point) =>
    sceneToGame(camera, gltfToScene(point)),
  );
  const top = triangles.filter((triangle) =>
    triangle.every((point) => Math.abs(point[2] - height) < 0.5),
  );
  assert.ok(top.length);
  const projected = top.map((triangle) => [triangle.map(([x, y]) => [x, y])]);
  const mesh = fixedPolygonBoolean(
    "union",
    [projected[0]],
    projected.slice(1).map((polygon) => [polygon]),
  );
  const clipped = fixedPolygonBoolean(
    "intersection",
    [[surface.polygon, ...(surface.holes ?? [])]],
    [mesh],
  );
  assert.equal(clipped.length, 1, `${surface.id}: review split deck before proceeding`);
  const [outer, ...holes] = clipped[0];
  changes.push({ surface: surface.id, before: surface.polygon, after: outer, holes });
  surface.polygon = outer.slice(0, -1);
  surface.holes = holes.map((hole) => hole.slice(0, -1));
  surface.height = surface.polygon.map(() => height);
  surface.navigationJoins = surface.polygon.map(([x, y], index) => [
    [x, y, height],
    [...surface.polygon[(index + 1) % surface.polygon.length], height],
  ]);
}
validateAssetGameplay(edit.gameplay, descriptor);
const output = await fs.mkdtemp("work/needs-review/map-compile/drawbridge-mesh-footprints-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(
  `${output}/mesh-footprints.json`,
  JSON.stringify({ stage, published: false, changes }),
);
console.log(output);
