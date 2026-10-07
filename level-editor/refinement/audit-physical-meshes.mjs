// Asset-only topology preflight for authoring missing physical gameplay volumes.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh } from "../pipeline/src/mask-recovery-mesh.ts";
import { closedMeshComponents } from "../pipeline/src/closed-mesh-components.ts";
import { sceneToGame, gltfToScene } from "../shared/src/geometry.ts";

const ids = process.argv.slice(2);
assert.ok(ids.length && ids.every((id) => !id.startsWith("-")), "Supply library asset IDs");
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json")).assets;
const camera = { kind: "oblique-orthographic", elevation_deg: 35 };
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const output = await fs.mkdtemp("work/map-compile/physical-mesh-audit-");
const results = [];
for (const id of ids) {
  const entry = index.find((entry) => entry.id === id);
  assert.ok(entry, `Missing asset ${id}`);
  const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
  assert.equal(hash(bytes), entry.descriptor_sha256);
  const descriptor = JSON.parse(bytes);
  const reference = {
    id,
    role: "objects",
    model: `3d-assets/${entry.model}`,
    model_sha256: hash(await fs.readFile(`library/3d-assets/${entry.model}`)),
    descriptor: `3d-assets/${entry.descriptor}`,
    descriptor_sha256: hash(bytes),
    model_scene: entry.model_scene,
    resources: descriptor.resources ?? [],
  };
  const model = await loadSceneModel("library", reference);
  const parts = [];
  for (const part of descriptor.parts) {
    let triangles;
    try {
      triangles = maskRecoveryMesh(model, part.node, (p) => sceneToGame(camera, gltfToScene(p)));
      const components = closedMeshComponents(triangles);
      parts.push({
        node: part.node,
        triangles: triangles.length,
        closedComponents: components.map((component) => ({
          triangles: component.length,
          min: [0, 1, 2].map((axis) =>
            component.reduce(
              (n, triangle) => Math.min(n, ...triangle.map((p) => p[axis])),
              Infinity,
            ),
          ),
          max: [0, 1, 2].map((axis) =>
            component.reduce(
              (n, triangle) => Math.max(n, ...triangle.map((p) => p[axis])),
              -Infinity,
            ),
          ),
        })),
      });
    } catch (error) {
      parts.push({ node: part.node, triangles: triangles?.length, error: String(error) });
    }
  }
  results.push({
    id,
    model_sha256: reference.model_sha256,
    descriptor_sha256: reference.descriptor_sha256,
    parts,
  });
}
await fs.writeFile(
  `${output}/report.json`,
  JSON.stringify(
    {
      scope: "topology-preflight-only-not-solid-or-gameplay-certification",
      results,
    },
    null,
    2,
  ),
);
console.log(JSON.stringify({ output, results }, null, 2));
