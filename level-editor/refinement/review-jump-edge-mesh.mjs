// Measure an authored jump edge against its asset's physical walking mesh.
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
const [map, assetId, segmentId, stage] = process.argv.slice(2);
assert.ok(
  map && assetId && segmentId,
  "Supply map, asset ID, external jump segment ID and optional stage",
);
const scene = await readStoredMap(`library/scenes/${map}.rhlos-map.json`, "library");
const source = scene.assetSources.find((source) => source.id === assetId);
assert.ok(source, "Asset must be pinned in the scene");
const bytes = await fs.readFile(`library/${source.descriptor}`);
assert.equal(createHash("sha256").update(bytes).digest("hex"), source.descriptor_sha256);
const descriptor = JSON.parse(bytes);
if (stage) {
  const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`));
  const edit = edits.find((edit) => edit.asset === assetId);
  assert.ok(edit, "Staged asset must exist");
  assert.equal(edit.descriptorSha256, source.descriptor_sha256);
  assert.equal(edit.modelSha256, source.model_sha256);
  descriptor.gameplay = edit.gameplay;
}
const segment = descriptor.gameplay.jumpSegments.find((segment) => segment.id === segmentId);
assert.ok(segment, "External jump segment must exist");
const model = await loadSceneModel("library", source);
const nodes = [...new Set(descriptor.gameplay.surfaces.map((surface) => surface.node))];
const meshes = nodes.map((node) => ({
  node,
  mesh: maskRecoveryMesh(model, node, (point) => sceneToGame(scene.camera, gltfToScene(point))),
}));
const triangles = meshes.flatMap(({ node, mesh }) =>
  mesh.map((points) => ({ node, triangle: new Triangle(...points.map((p) => new Vector3(...p))) })),
);
assert.ok(triangles.length, "Asset must have walking mesh triangles");
const samples = Array.from({ length: 17 }, (_, step) => {
  const point = segment.edge.a.map((n, axis) => n + ((segment.edge.b[axis] - n) * step) / 16);
  const vector = new Vector3(...point);
  const closest = new Vector3();
  let nearest;
  const surfacesAtXY = [];
  for (const { node, triangle } of triangles) {
    triangle.closestPointToPoint(vector, closest);
    const distance = vector.distanceTo(closest);
    if (!nearest || distance < nearest.distance)
      nearest = { node, distance, point: closest.toArray() };
    const { a, b, c } = triangle;
    const determinant = (b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y);
    if (Math.abs(determinant) < 1e-8) continue;
    const u = ((point[0] - a.x) * (c.y - a.y) - (c.x - a.x) * (point[1] - a.y)) / determinant;
    const v = ((b.x - a.x) * (point[1] - a.y) - (point[0] - a.x) * (b.y - a.y)) / determinant;
    if (Math.min(u, v, 1 - u - v) < -1e-7) continue;
    const height = a.z + u * (b.z - a.z) + v * (c.z - a.z);
    surfacesAtXY.push({ node, height, heightAboveEdge: height - point[2] });
  }
  return { point, nearest, surfacesAtXY };
});
const output = await fs.mkdtemp("work/map-compile/jump-edge-mesh-review-");
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({
    scope: "asset-local-mesh-distance-not-movement-or-rendering-proof",
    map,
    assetId,
    segmentId,
    stage,
    source,
    segment,
    meshes,
    samples,
  }),
);
console.log(output);
console.log(
  JSON.stringify(
    samples.filter((_, i) => [0, 8, 16].includes(i)),
    null,
    2,
  ),
);
