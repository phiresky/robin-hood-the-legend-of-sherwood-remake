// Inspect visible mesh heights at an authored world-space landing or floor point.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { readStoredMap } from "../pipeline/src/stored-map.ts";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh } from "../pipeline/src/mask-recovery-mesh.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { gltfToScene, sceneToGame, applyAffineMatrix } from "../shared/src/geometry.ts";

const args = process.argv.slice(2);
const libraryFlags = args.filter((arg) => arg.startsWith("--library="));
assert.ok(libraryFlags.length <= 1, "Supply at most one library root");
const library = libraryFlags[0]?.slice("--library=".length) ?? "library";
assert.ok(library);
const [map, xText, yText, zText, ...assets] = args.filter((arg) => !libraryFlags.includes(arg));
const point = [xText, yText, zText].map(Number);
assert.ok(
  map && assets.length && point.every(Number.isFinite),
  "Supply map, world X/Y/Z and asset IDs",
);
const document = await readStoredMap(`${library}/scenes/${map}.rhlos-map.json`, library);
const results = [];
for (const asset of assets) {
  const source = document.assetSources.find((source) => source.id === asset);
  assert.ok(source, `Missing asset ${asset}`);
  const model = await loadSceneModel(library, source);
  for (const object of document.objects.filter((object) =>
    object.node.startsWith(`asset:${asset}:`),
  )) {
    const matrix = partMatrix(document.camera, document, object);
    const triangles = maskRecoveryMesh(model, object.node.slice(`asset:${asset}:`.length), (p) =>
      sceneToGame(document.camera, applyAffineMatrix(matrix, gltfToScene(p))),
    );
    const hits = [];
    let nearest = null;
    for (const triangle of triangles) {
      const [a, b, c] = triangle;
      const det = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
      if (Math.abs(det) > 1e-8) {
        const u = ((b[1] - c[1]) * (point[0] - c[0]) + (c[0] - b[0]) * (point[1] - c[1])) / det;
        const v = ((c[1] - a[1]) * (point[0] - c[0]) + (a[0] - c[0]) * (point[1] - c[1])) / det;
        if (u >= -1e-8 && v >= -1e-8 && u + v <= 1 + 1e-8)
          hits.push({ height: u * a[2] + v * b[2] + (1 - u - v) * c[2], triangle });
      }
      if (!triangle.every((p) => Math.abs(p[2] - point[2]) < 0.5)) continue;
      for (const [i, a] of triangle.entries()) {
        const b = triangle[(i + 1) % 3];
        const dx = b[0] - a[0],
          dy = b[1] - a[1];
        const t = Math.max(
          0,
          Math.min(1, ((point[0] - a[0]) * dx + (point[1] - a[1]) * dy) / (dx * dx + dy * dy || 1)),
        );
        const closest = a.map((n, axis) => n + t * (b[axis] - n));
        const distance = Math.hypot(closest[0] - point[0], closest[1] - point[1]);
        if (!nearest || distance < nearest.distance) nearest = { distance, closest, triangle };
      }
    }
    hits.sort((a, b) => Math.abs(a.height - point[2]) - Math.abs(b.height - point[2]));
    results.push({
      asset,
      node: object.node,
      descriptorSha256: source.descriptor_sha256,
      modelSha256: source.model_sha256,
      hits,
      nearestLevelEdge: nearest,
    });
  }
}
const output = await fs.mkdtemp("work/needs-review/map-compile/placed-point-support-");
await fs.writeFile(
  `${output}/report.json`,
  JSON.stringify({ library, map, point, results }, null, 2),
);
console.log(output);
for (const result of results)
  console.log(
    JSON.stringify({
      asset: result.asset,
      node: result.node,
      heights: result.hits.map((hit) => hit.height),
      nearestLevelEdge: result.nearestLevelEdge?.distance,
    }),
  );
