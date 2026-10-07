// Compare authored climbing planes with explicitly selected mesh parts.
// This measures alignment only; a climbing contact need not lie inside the mesh.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh } from "../pipeline/src/mask-recovery-mesh.ts";
import { sceneToGame, gltfToScene } from "../shared/src/geometry.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";

const [asset, liftId, nodePattern, staged] = process.argv.slice(2);
assert.ok(
  asset && liftId && nodePattern && process.argv.length <= 6,
  "Usage: node refinement/audit-climb-mesh.mjs ASSET LIFT NODE_REGEX [STAGED_DIRECTORY]",
);
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const entries = JSON.parse(await fs.readFile("library/3d-assets/index.json")).assets;
const entry = entries.find((entry) => entry.id === asset);
assert.ok(entry, `Missing asset: ${asset}`);
const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
assert.equal(hash(bytes), entry.descriptor_sha256);
const descriptor = JSON.parse(bytes);
const stagedBytes = staged ? await fs.readFile(`${staged}/edits.json`) : undefined;
let gameplay = descriptor.gameplay;
if (stagedBytes) {
  const edit = JSON.parse(stagedBytes).find((edit) => edit.asset === asset);
  assert.ok(edit, "Staged edits do not contain this asset");
  assert.equal(edit.descriptorSha256, entry.descriptor_sha256);
  gameplay = edit.gameplay;
}
const lift = gameplay?.lifts?.find((lift) => lift.id === liftId);
assert.ok(lift && (lift.type === 2 || lift.type === 3), "Select a ladder or climbable wall");
const surface = gameplay.surfaces.find((surface) => surface.id === lift.surface);
assert.ok(surface, "Missing climbing surface");
const plane = heightPlane(surface.polygon.map(([x, y], i) =>
  [x, y, Array.isArray(surface.height) ? surface.height[i] : surface.height]));
const normalLength = Math.hypot(plane[0], plane[1], 1);
const signedDistance = (point) => (planeHeight(plane, point) - point[2]) / normalLength;
const reference = {
  id: asset,
  role: "objects",
  model: `3d-assets/${entry.model}`,
  model_sha256: hash(await fs.readFile(`library/3d-assets/${entry.model}`)),
  descriptor: `3d-assets/${entry.descriptor}`,
  descriptor_sha256: entry.descriptor_sha256,
  resources: descriptor.resources ?? [],
  ...(entry.model_scene ? { model_scene: entry.model_scene } : {}),
};
const model = await loadSceneModel("library", reference);
const pattern = new RegExp(nodePattern);
const selected = descriptor.parts.filter((part) => pattern.test(part.node));
assert.ok(selected.length, "No mesh parts match NODE_REGEX");
const camera = { kind: "oblique-orthographic", elevation_deg: 35 };
const parts = selected.map((part) => {
  const triangles = maskRecoveryMesh(model, part.node,
    (point) => sceneToGame(camera, gltfToScene(point)));
  assert.ok(triangles.length, `Empty mesh: ${part.node}`);
  const points = triangles.flat();
  const min = [0, 1, 2].map((axis) => points.reduce((n, p) => Math.min(n, p[axis]), Infinity));
  const max = [0, 1, 2].map((axis) => points.reduce((n, p) => Math.max(n, p[axis]), -Infinity));
  const center = min.map((n, i) => (n + max[i]) / 2);
  const distances = points.map(signedDistance);
  const distanceRange = [
    distances.reduce((a, b) => Math.min(a, b), Infinity),
    distances.reduce((a, b) => Math.max(a, b), -Infinity),
  ];
  return {
    node: part.node, triangles: triangles.length, min, max, boundsCenter: center,
    signedPlaneDistanceRange: distanceRange,
    boundsCenterPlaneDistance: signedDistance(center),
    minimumVertexPlaneDistance: distances.reduce((a, b) => Math.min(a, Math.abs(b)), Infinity),
    crossesInfinitePlane: triangles.some((triangle) => {
      const d = triangle.map(signedDistance);
      return Math.min(...d) <= 0 && Math.max(...d) >= 0;
    }),
  };
});
const output = await fs.mkdtemp("work/map-compile/climb-mesh-audit-");
await fs.writeFile(`${output}/report.json`, JSON.stringify({
  scope: "mesh-plane-alignment-only-not-contact-or-traversal-certification",
  asset, lift: liftId, nodePattern, reference,
  ...(stagedBytes ? { staged, stagedSha256: hash(stagedBytes) } : {}),
  plane, surface, doors: lift.doors, parts,
}, null, 2));
console.log(JSON.stringify({
  output, plane, parts: parts.length,
  partsCrossingPlane: parts.filter((part) => part.crossesInfinitePlane).length,
  maximumBoundsCenterPlaneDistance: Math.max(...parts.map((part) => Math.abs(part.boundsCenterPlaneDistance))),
}, null, 2));
