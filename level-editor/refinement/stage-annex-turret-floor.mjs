// Diagnose a discarded turret floor using its retained asset-local receiver.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh } from "../pipeline/src/mask-recovery-mesh.ts";
import { sceneToGame, gltfToScene, signedPolygonArea } from "../shared/src/geometry.ts";
import { fixedPolygonBoolean } from "../shared/src/fixed-polygon-boolean.ts";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";

const [meshOption] = process.argv.slice(2);
assert.ok(process.argv.length <= 3 && (!meshOption || meshOption.startsWith("--mesh-library=")));
const meshLibrary = meshOption?.slice("--mesh-library=".length);
const scene = JSON.parse(await fs.readFile("library/scenes/lincoln.rhlos-map.json"));
const edits = [];
let meshReview;
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
for (const asset of ["lincoln-keep-annex-round-turret", "lincoln-keep-annex"]) {
  const source = scene.assetSources.find((entry) => entry.id === asset);
  assert.ok(source);
  const bytes = await fs.readFile(`library/${source.descriptor}`);
  assert.equal(hash(bytes), source.descriptor_sha256);
  assert.equal(hash(await fs.readFile(`library/${source.model}`)), source.model_sha256);
  const descriptor = JSON.parse(bytes);
  const gameplay = structuredClone(descriptor.gameplay);
  if (asset.endsWith("round-turret")) {
    assert.equal(gameplay.surfaces.length, 0);
    const part = descriptor.parts.find((part) => part.node === "building-228");
    assert.ok(part?.obstacle_local_game?.projection_area);
    const points = part.obstacle_local_game.points;
    assert.ok(points.every((point) => Math.abs(point.z_top - 472.00104) < 1e-6));
    let polygon = points.map((point) => [point.x, point.y]);
    if (meshLibrary) {
      const stagedScene = JSON.parse(
        await fs.readFile(`${meshLibrary}/scenes/lincoln.rhlos-map.json`),
      );
      const stagedSource = stagedScene.assetSources.find((entry) => entry.id === asset);
      assert.equal(stagedSource?.descriptor_sha256, source.descriptor_sha256);
      const model = await loadSceneModel(meshLibrary, {
        ...stagedSource,
        resources: descriptor.resources ?? [],
        model_scene: descriptor.model_scene,
      });
      const triangles = maskRecoveryMesh(model, part.node, (point) =>
        sceneToGame(scene.camera, gltfToScene(point)),
      ).filter((triangle) =>
        triangle.every((point) => Math.abs(point[2] - points[0].z_top) < 0.001),
      );
      assert.ok(triangles.length, "Staged model has no floor at the retained height");
      const projected = triangles.map((triangle) => [triangle.map(([x, y]) => [x, y])]);
      const mesh = fixedPolygonBoolean("union", projected[0], projected.slice(1));
      const clipped = fixedPolygonBoolean("intersection", [polygon], [mesh]);
      assert.equal(clipped.length, 1, "Turret floor unexpectedly split");
      assert.equal(clipped[0].length, 1, "Turret floor unexpectedly acquired holes");
      const beforeArea = Math.abs(signedPolygonArea(polygon));
      polygon = clipped[0][0].slice(0, -1);
      meshReview = {
        library: meshLibrary,
        modelSha256: stagedSource.model_sha256,
        triangles: triangles.length,
        beforeArea,
        afterArea: Math.abs(signedPolygonArea(polygon)),
        scope: "unpublished-gameplay-for-staged-mesh-not-installed-model",
      };
      assert.ok(meshReview.afterArea > 0 && meshReview.afterArea < beforeArea);
      let samples = 0;
      const unsupported = [];
      for (
        let x = Math.floor(Math.min(...polygon.map((p) => p[0])));
        x <= Math.max(...polygon.map((p) => p[0]));
        x += 2
      ) {
        for (
          let y = Math.floor(Math.min(...polygon.map((p) => p[1])));
          y <= Math.max(...polygon.map((p) => p[1]));
          y += 2
        ) {
          if (!pointInGameplayPolygon([x, y], polygon)) continue;
          samples++;
          const supported = triangles.some(([a, b, c]) => {
            const det = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
            if (Math.abs(det) < 1e-8) return false;
            const u = ((b[1] - c[1]) * (x - c[0]) + (c[0] - b[0]) * (y - c[1])) / det;
            const v = ((c[1] - a[1]) * (x - c[0]) + (a[0] - c[0]) * (y - c[1])) / det;
            return (
              u >= -1e-8 &&
              v >= -1e-8 &&
              u + v <= 1 + 1e-8 &&
              Math.abs(u * a[2] + v * b[2] + (1 - u - v) * c[2] - points[0].z_top) < 0.001
            );
          });
          if (!supported) unsupported.push([x, y]);
        }
      }
      meshReview.coverage = { samples, unsupported };
      assert.ok(samples > 0);
      assert.equal(unsupported.length, 0, "Clipped floor still extends beyond the mesh");
    }
    gameplay.surfaces.push({
      id: "building-228-walk-0",
      node: part.node,
      navigationRegion: "round-turret-floor",
      polygon,
      height: polygon.map(() => points[0].z_top),
      holes: [],
      preserveMovementPrecision: true,
    });
    gameplay.draft.issues.push(
      meshReview
        ? `Unpublished receiver clipped to staged mesh ${meshReview.modelSha256}; installed model is unchanged. Pair with the reviewed model and verify moved assemblies before publication.`
        : "Diagnostic receiver restoration only: retained floor height is 472.00104, but visible turret slabs overlap it at 476/484/486. Reconcile mesh geometry before publication.",
    );
  } else {
    const door = gameplay.doors.find((door) => door.id === "passage-1/door-0");
    assert.deepEqual(door.outsideAnchor, [-27, 61.88272614076818, 472.00104]);
    delete door.outsideAnchor;
    const transition = gameplay.movementTransitions.find((entry) => entry.id === "door-change-7");
    assert.deepEqual(transition.waypointAnchor, [-27, 61.88272614076818, 472.00104]);
    delete transition.waypointAnchor;
    gameplay.draft.issues.push(
      "Diagnostic gate binding: select the actual outside endpoint on the separate turret; verify control binding and moved assemblies before publication.",
    );
  }
  validateAssetGameplay(gameplay, descriptor);
  edits.push({
    asset,
    descriptorSha256: source.descriptor_sha256,
    modelSha256: source.model_sha256,
    gameplay,
  });
}
const output = await fs.mkdtemp("work/needs-review/map-compile/annex-turret-floor-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
if (meshReview)
  await fs.writeFile(`${output}/mesh-review.json`, JSON.stringify(meshReview, null, 2));
console.log(output);
