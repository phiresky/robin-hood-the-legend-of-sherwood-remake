// Inspect visible mesh contact along an authored passage without changing collision.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs/promises";
import { readStoredMap } from "../pipeline/src/stored-map.ts";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh } from "../pipeline/src/mask-recovery-mesh.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { gltfToScene, sceneToGame, applyAffineMatrix } from "../shared/src/geometry.ts";

const [map, asset, ...coordinates] = process.argv.slice(2);
assert.ok(map && asset && coordinates.length === 6, "Supply map, asset and two world XYZ points");
const points = coordinates.map(Number);
assert.ok(points.every(Number.isFinite));
const start = points.slice(0, 3),
  end = points.slice(3);
const sub = (a, b) => a.map((n, i) => n - b[i]);
const dot = (a, b) => a.reduce((sum, n, i) => sum + n * b[i], 0);
const cross = (a, b) => [
  a[1] * b[2] - a[2] * b[1],
  a[2] * b[0] - a[0] * b[2],
  a[0] * b[1] - a[1] * b[0],
];
const direction = sub(end, start);
assert.ok(dot(direction, direction) > 0, "Segment must have positive length");
const scenePath = `library/scenes/${map}.rhlos-map.json`;
const sceneBytes = await fs.readFile(scenePath);
const document = await readStoredMap(scenePath, "library");
assert.ok(sceneBytes.equals(await fs.readFile(scenePath)), "Scene changed during inspection");
const source = document.assetSources.find((entry) => entry.id === asset);
assert.ok(source, `Missing asset ${asset}`);
const model = await loadSceneModel("library", source);
const results = [];
for (const object of document.objects.filter((entry) => entry.node.startsWith(`asset:${asset}:`))) {
  const matrix = partMatrix(document.camera, document, object);
  const triangles = maskRecoveryMesh(model, object.node.slice(`asset:${asset}:`.length), (p) =>
    sceneToGame(document.camera, applyAffineMatrix(matrix, gltfToScene(p))),
  );
  const hits = [];
  for (const [index, triangle] of triangles.entries()) {
    const [a, b, c] = triangle;
    const edge1 = sub(b, a),
      edge2 = sub(c, a);
    const p = cross(direction, edge2),
      determinant = dot(edge1, p);
    if (Math.abs(determinant) < 1e-8) continue;
    const s = sub(start, a),
      u = dot(s, p) / determinant;
    const q = cross(s, edge1),
      v = dot(direction, q) / determinant,
      t = dot(edge2, q) / determinant;
    if (u < -1e-8 || v < -1e-8 || u + v > 1 + 1e-8 || t < 0 || t > 1) continue;
    hits.push({ index, t, point: start.map((n, i) => n + t * direction[i]), triangle });
  }
  hits.sort((a, b) => a.t - b.t);
  results.push({ node: object.node, hits });
}
const output = await fs.mkdtemp("work/needs-review/map-compile/placed-segment-contact-");
await fs.writeFile(
  `${output}/report.json`,
  JSON.stringify(
    {
      scope: "mesh-segment-contact-not-standing-box-clearance-or-gameplay-proof",
      map,
      sceneSha256: createHash("sha256").update(sceneBytes).digest("hex"),
      asset,
      start,
      end,
      descriptorSha256: source.descriptor_sha256,
      modelSha256: source.model_sha256,
      results,
    },
    null,
    2,
  ),
);
console.log(output);
for (const result of results.filter((entry) => entry.hits.length))
  console.log(JSON.stringify({ node: result.node, contacts: result.hits.map((hit) => hit.point) }));
