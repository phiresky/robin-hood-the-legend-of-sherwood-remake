// Compare staged gameplay boundaries with pinned model geometry before publication.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { createRequire } from "node:module";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh } from "../pipeline/src/mask-recovery-mesh.ts";
import { readStoredMap } from "../pipeline/src/stored-map.ts";
import { sceneToGame, gltfToScene } from "../shared/src/geometry.ts";

const { Triangle, Vector3 } = createRequire(new URL("../shared/package.json", import.meta.url))(
  "three",
);
const [stage] = process.argv.slice(2);
assert.ok(stage);
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`));
const scene = await readStoredMap("library/scenes/nottingham.rhlos-map.json", "library");
const output = await fs.mkdtemp("work/map-compile/roof-slab-mesh-review-");
const results = [];
for (const [id, surfaceId] of [
  ["nottingham-north-street-roof-edge-slab", "building-140-walk-0"],
  ["nottingham-north-stone-house", "building-113-walk-0"],
  ["nottingham-upper-green-house", "building-159-walk-0"],
]) {
  const source = scene.assetSources.find((source) => source.id === id);
  const edit = edits.find((edit) => edit.asset === id);
  assert.ok(source && edit);
  const bytes = await fs.readFile(`library/${source.descriptor}`);
  assert.equal(createHash("sha256").update(bytes).digest("hex"), edit.descriptorSha256);
  assert.equal(source.model_sha256, edit.modelSha256);
  const descriptor = JSON.parse(bytes);
  const model = await loadSceneModel("library", source);
  const surface = edit.gameplay.surfaces.find((surface) => surface.id === surfaceId);
  assert.ok(surface);
  const mesh = maskRecoveryMesh(model, surface.node, (p) =>
    sceneToGame(scene.camera, gltfToScene(p)),
  );
  assert.ok(mesh.length);
  const triangles = mesh.map((triangle) => new Triangle(...triangle.map((p) => new Vector3(...p))));
  const points = (surface) =>
    surface.polygon.map(([x, y], i) => [
      x,
      y,
      Array.isArray(surface.height) ? surface.height[i] : surface.height,
    ]);
  const before = points(descriptor.gameplay.surfaces.find((surface) => surface.id === surfaceId));
  const after = points(surface);
  const measure = (boundary) =>
    boundary.flatMap((a, i) => {
      const b = boundary[(i + 1) % boundary.length];
      return Array.from({ length: 17 }, (_, step) => {
        const point = a.map((n, axis) => n + ((b[axis] - n) * step) / 16);
        const p = new Vector3(...point),
          closest = new Vector3();
        let distance = Infinity,
          nearest;
        for (const triangle of triangles) {
          triangle.closestPointToPoint(p, closest);
          const candidate = p.distanceTo(closest);
          if (candidate < distance) {
            distance = candidate;
            nearest = closest.toArray();
          }
        }
        return { point, distance, nearest };
      });
    });
  const original = measure(before),
    staged = measure(after);
  results.push({
    id,
    surfaceId,
    descriptorSha256: edit.descriptorSha256,
    modelSha256: edit.modelSha256,
    mesh,
    before,
    after,
    original,
    staged,
    originalMaximumDistance: Math.max(...original.map((p) => p.distance)),
    stagedMaximumDistance: Math.max(...staged.map((p) => p.distance)),
  });
}
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({
    stage,
    scope: "mesh-boundary-distance-not-textured-render-or-gameplay-proof",
    results,
  }),
);
console.log(output);
console.log(
  JSON.stringify(
    results.map(({ id, originalMaximumDistance, stagedMaximumDistance }) => ({
      id,
      originalMaximumDistance,
      stagedMaximumDistance,
    })),
    null,
    2,
  ),
);
