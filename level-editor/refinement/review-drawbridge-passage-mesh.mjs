// Sample exported passage approaches against the placed asset meshes.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh } from "../pipeline/src/mask-recovery-mesh.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { gltfToScene, sceneToGame, applyAffineMatrix } from "../shared/src/geometry.ts";

const [directory] = process.argv.slice(2);
assert.ok(directory);
const manifest = JSON.parse(await fs.readFile(`${directory}/diagnostics.json`));
assert.equal(manifest.complete, true);
const models = new Map();
function height([x, y], [a, b, c]) {
  const determinant = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
  if (Math.abs(determinant) < 1e-8) return undefined;
  const u = ((b[1] - c[1]) * (x - c[0]) + (c[0] - b[0]) * (y - c[1])) / determinant;
  const v = ((c[1] - a[1]) * (x - c[0]) + (a[0] - c[0]) * (y - c[1])) / determinant;
  return u >= -1e-7 && v >= -1e-7 && u + v <= 1 + 1e-7
    ? u * a[2] + v * b[2] + (1 - u - v) * c[2]
    : undefined;
}
const results = [];
function edgeDistance(point, a, b) {
  const dx = b[0] - a[0],
    dy = b[1] - a[1];
  const t = Math.max(
    0,
    Math.min(1, ((point[0] - a[0]) * dx + (point[1] - a[1]) * dy) / (dx * dx + dy * dy || 1)),
  );
  return Math.hypot(point[0] - a[0] - t * dx, point[1] - a[1] - t * dy);
}
for (const entry of manifest.results) {
  const document = JSON.parse(await fs.readFile(`${directory}/${entry.file}.scene.json`));
  const descriptor = JSON.parse(await fs.readFile(`${directory}/${entry.file}`));
  const meshes = [];
  for (const source of document.assetSources) {
    if (!models.has(source.id)) models.set(source.id, await loadSceneModel("library", source));
    for (const object of document.objects.filter((object) =>
      object.node.startsWith(`asset:${source.id}:`),
    )) {
      const node = object.node.slice(`asset:${source.id}:`.length);
      const matrix = partMatrix(document.camera, document, object);
      const triangles = maskRecoveryMesh(models.get(source.id), node, (point) =>
        sceneToGame(document.camera, applyAffineMatrix(matrix, gltfToScene(point))),
      );
      meshes.push({ object: object.id, triangles });
    }
  }
  const geometry = descriptor.asset_geometry;
  const combined = [
    ...(geometry.buildings ?? []).flatMap((building) => building.Building?.doors ?? []),
    ...geometry.doors,
  ];
  const passages = entry.passage_controls
    .flatMap((control) => control.doors)
    .map((index) => ({ index, passage: combined[index] }));
  assert.equal(passages.length, 12);
  for (const { index: door, passage } of passages) {
    assert.ok(passage.world_endpoints, `Missing ordinary passage ${door}`);
    const { outside, middle, inside } = passage.world_endpoints;
    const samples = [];
    for (const [a, b] of [
      [outside, middle],
      [middle, inside],
    ]) {
      for (let step = 0; step <= 20; step++) {
        const point = a.map((value, axis) => value + ((b[axis] - value) * step) / 20);
        let nearest = Infinity;
        let gap = Infinity;
        const owners = [];
        for (const mesh of meshes) {
          let error = Infinity;
          for (const triangle of mesh.triangles) {
            const z = height(point, triangle);
            if (z !== undefined) error = Math.min(error, Math.abs(z - point[2]));
            if (z !== undefined && Math.abs(z - point[2]) <= 0.5) gap = 0;
            else if (gap > 0 && triangle.every((vertex) => Math.abs(vertex[2] - point[2]) <= 0.5)) {
              gap = Math.min(
                gap,
                ...triangle.map((a, index) => edgeDistance(point, a, triangle[(index + 1) % 3])),
              );
            }
          }
          nearest = Math.min(nearest, error);
          if (error <= 0.5) owners.push(mesh.object);
        }
        samples.push({
          point,
          supported: owners.length > 0,
          owners,
          heightError: Number.isFinite(nearest) ? nearest : null,
          horizontalGap: Number.isFinite(gap) ? gap : null,
        });
      }
    }
    results.push({ file: entry.file, door, samples });
  }
  console.log(`${entry.file}: sampled twelve passage approaches`);
}
const complete = results.every((result) => result.samples.every((sample) => sample.supported));
await fs.writeFile(
  `${directory}/passage-mesh-report.json`,
  JSON.stringify({
    scope: "sampled-exported-passage-centerlines-not-actor-footprint-or-animation",
    complete,
    results,
  }),
);
assert.ok(complete, "Passage mesh gaps: inspect passage-mesh-report.json");
