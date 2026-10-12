// Compare staged wall collision occupancy with the serialized repaired mesh.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh } from "../pipeline/src/mask-recovery-mesh.ts";
import { gltfToScene, sceneToGame } from "../shared/src/geometry.ts";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";

const [stage] = process.argv.slice(2);
assert.ok(stage);
const review = JSON.parse(await fs.readFile(`${stage}/review.json`));
const config = JSON.parse(await fs.readFile(`${review.modelStage}/opening.json`));
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`));
const gameplay = edits.find((entry) => entry.asset === config.asset).gameplay;
const model = await loadSceneModel(review.modelStage, {
  id: config.asset,
  role: "objects",
  model: "model.glb",
  model_sha256: review.modelSha256,
  resources: [],
});
function rayHeight([x, y], [a, b, c]) {
  const d = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
  if (Math.abs(d) < 1e-10) return undefined;
  const u = ((b[1] - c[1]) * (x - c[0]) + (c[0] - b[0]) * (y - c[1])) / d;
  const v = ((c[1] - a[1]) * (x - c[0]) + (a[0] - c[0]) * (y - c[1])) / d;
  return u >= 0 && v >= 0 && u + v <= 1 ? u * a[2] + v * b[2] + (1 - u - v) * c[2] : undefined;
}
assert.equal(
  rayHeight(
    [0.25, 0.25],
    [
      [0, 0, 10],
      [1, 0, 10],
      [0, 1, 10],
    ],
  ),
  10,
);
assert.equal(
  rayHeight(
    [2, 2],
    [
      [0, 0, 10],
      [1, 0, 10],
      [0, 1, 10],
    ],
  ),
  undefined,
);
const results = [];
for (const node of config.nodes) {
  const mesh = maskRecoveryMesh(model, node, (p) => sceneToGame(config.camera, gltfToScene(p)));
  const volumes = gameplay.volumes
    .filter((volume) => volume.node === node)
    .map(({ shape }) => ({
      polygon: shape.points.map((p) => [p.x, p.y]),
      bottom: heightPlane(shape.points.map((p) => [p.x, p.y, p.z_bottom])),
      top: heightPlane(shape.points.map((p) => [p.x, p.y, p.z_top])),
    }));
  let probes = 0,
    mismatches = 0,
    openingProbes = 0,
    openingMismatches = 0;
  const examples = [];
  for (let x = -42.123; x < 40; x += 1)
    for (let y = -49.237; y < 15; y += 1) {
      const point = [x, y];
      const hits = mesh
        .map((triangle) => rayHeight(point, triangle))
        .filter((height) => height !== undefined)
        .sort((a, b) => a - b)
        .filter((height, i, values) => i === 0 || Math.abs(height - values[i - 1]) > 1e-5);
      assert.equal(hits.length % 2, 0, `${node}: ambiguous ray at ${point}`);
      const local = volumes.filter((volume) => pointInGameplayPolygon(point, volume.polygon));
      const heights = Array.from({ length: 33 }, (_, i) => 310.137 + i * 2);
      heights.push(
        config.floorHeight - 0.01,
        config.floorHeight + 0.01,
        355.995,
        356.005,
        config.doorwayTop - 0.001,
        config.doorwayTop + 0.001,
      );
      for (const z of heights) {
        const meshSolid = hits.filter((height) => height < z).length % 2 === 1;
        const authoredSolid = local.some(
          (volume) => z > planeHeight(volume.bottom, point) && z < planeHeight(volume.top, point),
        );
        const opening =
          pointInGameplayPolygon(point, config.polygon) &&
          z > config.floorHeight + 0.05 &&
          z < config.doorwayTop - 0.05;
        probes++;
        if (opening) openingProbes++;
        if (meshSolid !== authoredSolid) {
          mismatches++;
          if (opening) openingMismatches++;
          if (examples.length < 20)
            examples.push({ point: [x, y, z], meshSolid, authoredSolid, opening });
        }
      }
    }
  results.push({ node, probes, mismatches, openingProbes, openingMismatches, examples });
}
const report = { scope: "sampled-asset-local-wall-occupancy-not-actor-clearance", results };
await fs.writeFile(`${stage}/wall-occupancy.json`, JSON.stringify(report));
console.log(JSON.stringify(report));
if (results.some((result) => result.mismatches > 0)) process.exitCode = 1;
