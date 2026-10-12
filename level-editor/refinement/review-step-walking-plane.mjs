// Validate the actual serialized stair top against its staged receiving plane.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh } from "../pipeline/src/mask-recovery-mesh.ts";
import { closedMeshComponents } from "../pipeline/src/closed-mesh-components.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { fixedPolygonBoolean } from "../shared/src/fixed-polygon-boolean.ts";
import { gltfToScene, sceneToGame, signedPolygonArea } from "../shared/src/geometry.ts";

const [stage] = process.argv.slice(2);
assert.ok(stage);
const config = JSON.parse(await fs.readFile(`${stage}/mesh-plane.json`));
const build = JSON.parse(await fs.readFile(`${stage}/mesh-build.json`));
const [edit] = JSON.parse(await fs.readFile(`${stage}/edits.json`));
const surface = edit.gameplay.surfaces.find((entry) => entry.node === config.node);
const model = await loadSceneModel(stage, {
  id: edit.asset,
  role: "objects",
  model: "model.glb",
  model_sha256: build.modelSha256,
  resources: [],
});
const mesh = maskRecoveryMesh(model, config.node, (p) =>
  sceneToGame(config.camera, gltfToScene(p)),
);
assert.equal(closedMeshComponents(mesh).length, 1);
const plane = heightPlane(surface.projectionMaterials.planePoints);
const minimum = Math.min(...surface.height);
const top = mesh.filter((triangle) => triangle.every((p) => p[2] >= minimum - 1e-4));
assert.equal(top.length, 3, "The five-corner step top must retain three triangles");
const maximumHeightError = Math.max(
  ...top.flatMap((triangle) => triangle.map((p) => Math.abs(p[2] - planeHeight(plane, p)))),
);
const shapes = top.map((triangle) => [triangle.map(([x, y]) => [x, y])]);
const union = fixedPolygonBoolean("union", shapes);
const difference = fixedPolygonBoolean("xor", union, [[surface.polygon]]);
const footprintDifferenceArea = difference.reduce(
  (sum, polygon) =>
    sum +
    Math.abs(signedPolygonArea(polygon[0])) -
    polygon.slice(1).reduce((holes, ring) => holes + Math.abs(signedPolygonArea(ring)), 0),
  0,
);
const maximumCornerError = Math.max(
  ...config.footprint.map((point) =>
    Math.min(
      ...top.flatMap((triangle) =>
        triangle.map((p) => Math.hypot(...p.map((v, i) => v - point[i]))),
      ),
    ),
  ),
);
const report = {
  scope: "serialized-step-plane-and-footprint-not-rendered-contact",
  modelSha256: build.modelSha256,
  maximumHeightError,
  maximumCornerError,
  footprintDifferenceArea,
};
await fs.writeFile(`${stage}/mesh-plane-review.json`, JSON.stringify(report));
console.log(JSON.stringify(report));
assert.ok(maximumHeightError < 1e-4);
assert.ok(maximumCornerError < 1e-4);
assert.ok(footprintDifferenceArea < 0.01);
