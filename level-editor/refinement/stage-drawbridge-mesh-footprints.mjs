// Keep the staged deck navigation within its visible near-horizontal mesh.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { createRequire } from "node:module";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh } from "../pipeline/src/mask-recovery-mesh.ts";
import { sceneToGame, gltfToScene } from "../shared/src/geometry.ts";
import { signedPolygonArea } from "../shared/src/geometry.ts";
import { fixedPolygonBoolean } from "../shared/src/fixed-polygon-boolean.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const [stage, mode, assetOption] = process.argv.slice(2);
assert.ok(stage, "Supply staged drawbridge and plateau edits");
assert.ok(mode === undefined || mode === "--plateau", "Unknown footprint review mode");
assert.ok(
  assetOption === undefined ||
    (mode === "--plateau" && /^--asset=[a-z0-9-]+\/[a-z0-9-]+$/.test(assetOption)),
  "Optional plateau asset must be --asset=<map>/<asset-id>",
);
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`));
const id =
  assetOption?.split("/").at(-1) ??
  (mode === "--plateau"
    ? "lincoln-castle-hill-inner-bailey-plateau"
    : "lincoln-south-gate-drawbridge");
const edit = edits.find((entry) => entry.asset === id);
assert.ok(edit);
const root = `3d-assets/${assetOption?.slice("--asset=".length) ?? `lincoln/${id}`}`;
const bytes = await fs.readFile(`library/${root}/asset.json`);
assert.equal(createHash("sha256").update(bytes).digest("hex"), edit.descriptorSha256);
const descriptor = JSON.parse(bytes);
assert.equal(descriptor.id, id);
const modelSha256 = createHash("sha256")
  .update(await fs.readFile(`library/${root}/${descriptor.model}`))
  .digest("hex");
if (edit.modelSha256 !== undefined) assert.equal(modelSha256, edit.modelSha256);
edit.modelSha256 = modelSha256;
const model = await loadSceneModel("library", {
  id: edit.asset,
  role: "objects",
  model: `${root}/${descriptor.model}`,
  model_sha256: edit.modelSha256,
  descriptor: `${root}/asset.json`,
  descriptor_sha256: edit.descriptorSha256,
  resources: descriptor.resources ?? [],
  ...(descriptor.model_scene ? { model_scene: descriptor.model_scene } : {}),
});
const camera = { kind: "oblique-orthographic", elevation_deg: 35 };
// Dense carved terrain contains nearly coincident tessellation edges. A local
// authoring grid below one thousandth of a game unit removes that mesh noise.
const clippingScale = mode === "--plateau" ? 1024 : 1048576;
const polygonClipping = createRequire(new URL("../shared/package.json", import.meta.url))(
  "polygon-clipping",
);
const changes = [];
const replacements = [];
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
  let batches = projected.map((polygon) => [polygon]);
  while (batches.length > 1) {
    console.log(`${surface.id}: union ${batches.length} mesh pieces`);
    const next = [];
    for (let index = 0; index < batches.length; index += 32) {
      next.push(
        mode === "--plateau"
          ? polygonClipping.union(...batches.slice(index, index + 32))
          : fixedPolygonBoolean(
              "union",
              batches[index],
              batches.slice(index + 1, index + 32),
              clippingScale,
            ),
      );
    }
    batches = next;
  }
  const mesh = batches[0];
  const clipped = fixedPolygonBoolean(
    "intersection",
    [[surface.polygon, ...(surface.holes ?? [])]],
    [mesh],
    clippingScale,
  );
  assert.ok(clipped.length, "The selected walking surface disappeared");
  if (mode !== "--plateau") assert.equal(clipped.length, 1, "Deck unexpectedly split");
  console.log(
    `${surface.id}: ${JSON.stringify(clipped.map(([outer]) => ({ area: Math.abs(signedPolygonArea(outer)), points: outer.length })))}`,
  );
  for (const [piece, [outer, ...holes]] of clipped.entries()) {
    const replacement = structuredClone(surface);
    replacement.id = piece === 0 ? surface.id : `${surface.id}-mesh-piece-${piece}`;
    changes.push({ surface: surface.id, before: surface.polygon, after: outer, holes });
    replacement.polygon = outer.slice(0, -1);
    replacement.holes = holes.map((hole) => hole.slice(0, -1));
    replacement.height = replacement.polygon.map(() => height);
    if (clipped.length > 1) {
      delete replacement.preserveMovementBoundary;
      if (replacement.projectionVolume !== undefined) {
        const volume = edit.gameplay.volumes?.find(
          (volume) => volume.id === replacement.projectionVolume,
        );
        assert.ok(volume && !volume.shape.material_indices?.length);
        delete replacement.projectionVolume;
        replacement.projectionMaterials = {
          defaultMaterial: volume.shape.default_material,
          regions: [],
        };
      } else assert.ok(replacement.projectionMaterials, "Split floor needs receiving materials");
    }
    if (replacement.navigationJoins)
      replacement.navigationJoins = replacement.polygon.map(([x, y], index) => [
        [x, y, height],
        [...replacement.polygon[(index + 1) % replacement.polygon.length], height],
      ]);
    replacements.push(replacement);
  }
}
edit.gameplay.surfaces = replacements;
validateAssetGameplay(edit.gameplay, descriptor);
const output = await fs.mkdtemp("work/needs-review/map-compile/drawbridge-mesh-footprints-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(
  `${output}/mesh-footprints.json`,
  JSON.stringify({ stage, mode, asset: id, modelSha256, clippingScale, published: false, changes }),
);
console.log(output);
