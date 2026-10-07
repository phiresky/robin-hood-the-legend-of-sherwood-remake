// Asset-only topology preflight for authoring missing physical gameplay volumes.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh } from "../pipeline/src/mask-recovery-mesh.ts";
import { closedMeshComponents } from "../pipeline/src/closed-mesh-components.ts";
import { meshCappedVolumes } from "../pipeline/src/mesh-capped-volumes.ts";
import { simplifyPhysicalShell } from "../pipeline/src/simplify-physical-shell.ts";
import { sceneToGame, gltfToScene } from "../shared/src/geometry.ts";

const capped = process.argv.includes("--caps");
const retainCollinear = process.argv.includes("--retain-collinear");
const simplifyOption = process.argv.find((arg) => arg.startsWith("--simplify="));
const simplifyError = simplifyOption
  ? Number(simplifyOption.slice("--simplify=".length))
  : undefined;
assert.ok(simplifyError === undefined || (Number.isFinite(simplifyError) && simplifyError >= 0));
const ids = process.argv
  .slice(2)
  .filter((arg) => arg !== "--caps" && arg !== "--retain-collinear" && arg !== simplifyOption);
assert.ok(ids.length && ids.every((id) => !id.startsWith("-")), "Supply library asset IDs");
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json")).assets;
const camera = { kind: "oblique-orthographic", elevation_deg: 35 };
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const output = await fs.mkdtemp("work/map-compile/physical-mesh-audit-");
const results = [];
console.log(JSON.stringify({ output }));
for (const id of ids) {
  console.log(JSON.stringify({ auditing: id }));
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
    const collinearFaces = [];
    const simplifications = [];
    try {
      triangles = maskRecoveryMesh(model, part.node, (p) => sceneToGame(camera, gltfToScene(p)));
      console.log(
        JSON.stringify({
          asset: id,
          node: part.node,
          phase: "topology",
          triangles: triangles.length,
        }),
      );
      let components = closedMeshComponents(
        triangles,
        1e-5,
        retainCollinear ? (face) => collinearFaces.push(face) : undefined,
      );
      if (simplifyError !== undefined) {
        const simplified = [];
        for (const [index, component] of components.entries()) {
          const reported = [];
          const candidate = await simplifyPhysicalShell(
            component,
            simplifyError,
            retainCollinear ? (face) => reported.push(face) : undefined,
          );
          console.log(
            JSON.stringify({
              asset: id,
              node: part.node,
              component: index,
              phase: "simplified",
              triangles: candidate.triangles.length,
              approximateError: candidate.error,
            }),
          );
          simplifications.push({
            component: index,
            sourceTriangles: component.length,
            triangles: candidate.triangles.length,
            approximateError: candidate.error,
            collinearFaces: reported,
          });
          simplified.push(candidate.triangles);
        }
        components = simplified;
      }
      let cappedVolumes;
      if (capped) {
        assert.ok(
          components.every((component) => component.length <= 5000),
          "Cap decomposition preflight is limited to 5000 triangles per shell; simplify the physical mesh first",
        );
        const componentCollinearFaces = [];
        cappedVolumes = components.flatMap((component, index) =>
          meshCappedVolumes(
            component,
            retainCollinear
              ? (face) => componentCollinearFaces.push({ component: index, face })
              : undefined,
          ),
        );
        if (simplifyError === undefined)
          assert.equal(componentCollinearFaces.length, collinearFaces.length);
        await fs.writeFile(
          `${output}/${id}-${parts.length}-caps.json`,
          JSON.stringify(cappedVolumes),
        );
      }
      parts.push({
        node: part.node,
        triangles: triangles.length,
        collinearFaces,
        simplifications,
        cappedVolumes: cappedVolumes?.length,
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
      parts.push({
        node: part.node,
        triangles: triangles?.length,
        collinearFaces,
        simplifications,
        error: String(error),
      });
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
      retainCollinearFaces: retainCollinear,
      requestedSimplificationError: simplifyError,
      results,
    },
    null,
    2,
  ),
);
console.log(
  JSON.stringify(
    {
      output,
      results: results.map((result) => ({
        id: result.id,
        parts: result.parts.map((part) => ({
          node: part.node,
          triangles: part.triangles,
          cappedVolumes: part.cappedVolumes,
          components: part.closedComponents?.length,
          error: part.error,
        })),
      })),
    },
    null,
    2,
  ),
);
