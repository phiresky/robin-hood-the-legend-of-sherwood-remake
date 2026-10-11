// Compare exported sight/projectile collision with samples from the pinned mesh.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh } from "../pipeline/src/mask-recovery-mesh.ts";
import { closedMeshComponents } from "../pipeline/src/closed-mesh-components.ts";
import { sceneToGame, gltfToScene } from "../shared/src/geometry.ts";
import { gameToScene } from "../shared/src/scene.ts";
import { partMatrix } from "../shared/src/level3d.ts";

const [stage] = process.argv.slice(2);
assert.ok(stage);
const manifest = JSON.parse(await fs.readFile(`${stage}/diagnostics.json`, "utf8"));
assert.equal(manifest.complete, true);
const cache = new Map();
for (const result of manifest.results) {
  const document = JSON.parse(await fs.readFile(`${stage}/${result.file}.scene.json`, "utf8"));
  const [reference] = document.assetSources;
  assert.equal(document.objects.length, 1, "This audit requires one physical wood part");
  const part = document.objects[0];
  const node = part.node.split(":").slice(2).join(":");
  let components = cache.get(reference.id);
  if (!components) {
    const model = await loadSceneModel("library", reference);
    components = closedMeshComponents(
      maskRecoveryMesh(model, node, (point) => sceneToGame(document.camera, gltfToScene(point))),
      1e-5,
      () => {},
    );
    cache.set(reference.id, components);
  }
  const vertices = components.flat(2);
  const minX = Math.min(...vertices.map((point) => point[0]));
  const maxX = Math.max(...vertices.map((point) => point[0]));
  const minY = Math.min(...vertices.map((point) => point[1]));
  const maxY = Math.max(...vertices.map((point) => point[1]));
  const maxZ = Math.max(...vertices.map((point) => point[2]));
  const matrix = partMatrix(document.camera, document, part);
  const place = (point) => {
    const local = gameToScene(document.camera, ...point);
    return sceneToGame(
      document.camera,
      [0, 1, 2].map(
        (row) =>
          matrix[row] * local[0] +
          matrix[4 + row] * local[1] +
          matrix[8 + row] * local[2] +
          matrix[12 + row],
      ),
    );
  };
  const ray_probes = [];
  for (let ix = 0; ix < 19; ix++)
    for (let iy = 0; iy < 19; iy++) {
      const x = minX + ((maxX - minX) * (ix + 0.37)) / 19;
      const y = minY + ((maxY - minY) * (iy + 0.61)) / 19;
      const intervals = [];
      for (const component of components) {
        const intersections = [];
        for (const [a, b, c] of component) {
          const determinant = (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]);
          if (Math.abs(determinant) < 1e-10) continue;
          const u = ((x - a[0]) * (c[1] - a[1]) - (y - a[1]) * (c[0] - a[0])) / determinant;
          const v = ((b[0] - a[0]) * (y - a[1]) - (b[1] - a[1]) * (x - a[0])) / determinant;
          if (u < 0 || v < 0 || u + v > 1) continue;
          intersections.push(a[2] + u * (b[2] - a[2]) + v * (c[2] - a[2]));
        }
        const heights = intersections
          .sort((a, b) => a - b)
          .filter((value, i, values) => i === 0 || value - values[i - 1] > 1e-5);
        assert.equal(heights.length % 2, 0, "Mesh ray must enter and leave each closed shell");
        for (let i = 0; i < heights.length; i += 2) intervals.push([heights[i], heights[i + 1]]);
      }
      const merged = [];
      for (const interval of intervals.sort((a, b) => a[0] - b[0])) {
        const previous = merged.at(-1);
        if (previous && previous[1] >= interval[0])
          previous[1] = Math.max(previous[1], interval[1]);
        else merged.push([...interval]);
      }
      const add = (bottom, top, clear) => {
        bottom = Math.max(0, bottom);
        if (top - bottom < 1) return;
        const z = (bottom + top) / 2;
        ray_probes.push({
          name: `mesh-${ix}-${iy}-${ray_probes.length}`,
          clear,
          // Blocking queries detect a surface crossing, not occupancy when both
          // endpoints are inside a solid. Cross out through the upper surface.
          endpoints: [place([x, y, z - 0.1]), place([x, y, clear ? z + 0.1 : maxZ + 2])],
        });
      };
      let bottom = 0;
      for (const interval of merged) {
        add(bottom, interval[0], true);
        add(interval[0], interval[1], false);
        bottom = Math.max(bottom, interval[1]);
      }
      add(bottom, maxZ + 2, true);
    }
  assert.ok(ray_probes.some((probe) => probe.clear) && ray_probes.some((probe) => !probe.clear));
  result.ray_probes = ray_probes;
  console.log(`${result.file}: ${ray_probes.length} mesh-derived probes`);
}
await fs.writeFile(`${stage}/diagnostics.json`, JSON.stringify(manifest));
