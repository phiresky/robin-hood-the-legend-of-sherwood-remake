import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh } from "../pipeline/src/mask-recovery-mesh.ts";
import { sceneToGame } from "../shared/src/geometry.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

// Review one terrain-owned contact against its pinned mesh. This stages asset
// metadata; it is not an export-time repair or permission to bridge other gaps.
const [staged, compiledFile] = process.argv.slice(2);
assert.ok(staged && compiledFile, "Provide stair edits and their compiled Derby descriptor");
const edits = JSON.parse(await fs.readFile(`${staged}/edits.json`, "utf8"));
assert.equal(edits.length, 1);
const contacts = {
  "derby-lower-west-access-stair": {
    outside: [429, 1775, 0],
    edge: [
      [407, 1768],
      [442, 1760],
    ],
  },
  "derby-east-hall": {
    outside: [1403, 1383, 0],
    edge: [
      [1375, 1381],
      [1411, 1365],
    ],
  },
  "derby-east-bailey-east-curtain": {
    outside: [1594, 1624, 0],
    edge: [
      [1581, 1609],
      [1629, 1619],
    ],
  },
};
const contact = contacts[edits[0].asset];
assert.ok(contact, "No reviewed terrain contact for this asset");
const bytes = await fs.readFile(compiledFile);
const compiled = JSON.parse(bytes).asset_geometry;
const matches = compiled.lifts.filter((lift) =>
  lift.physical_navigation?.doors.some(
    (door) =>
      Math.hypot(...door.outside.map((value, axis) => value - contact.outside[axis])) < 1e-5,
  ),
);
assert.equal(matches.length, 1, "Expected reviewed lower stair placement");
const plane = matches[0].physical_navigation.plane;
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
const entry = index.find((entry) => entry.id === "derby-terrain");
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const terrainBytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
assert.equal(hash(terrainBytes), entry.descriptor_sha256);
const descriptor = JSON.parse(terrainBytes);
const gameplay = structuredClone(descriptor.gameplay);
const surface = gameplay.surfaces.find((surface) => surface.id === "ground-section-0-0");
assert.ok(surface.height.every((z) => z === 0));
const before = contact.edge;
const positions = before.map((point) =>
  surface.polygon.findIndex((p) => p[0] === point[0] && p[1] === point[1]),
);
assert.ok(positions.every((i) => i >= 0));
assert.equal((positions[0] + 1) % surface.polygon.length, positions[1]);
// Prove that the compiled receiving boundary uses the same terrain-local frame.
const door = matches[0].doors[0];
const region = compiled.motion_data.layers[door.layer_out].find((area) =>
  before.every((p) => area.polygon.points.some((q) => p[0] === q[0] && p[1] === q[1])),
);
assert.ok(region, "Terrain frame no longer matches the reviewed placement");
const [a, b, c] = plane;
const after = before.map(([x, y]) => {
  const t = (a * x + b * y + c) / (a * a + b * b);
  assert.ok(Math.abs(t) * Math.hypot(a, b) < 2.5, "Contact needs broader review");
  return [x - t * a, y - t * b];
});
const modelBytes = await fs.readFile(`library/3d-assets/${entry.model}`);
const model = await loadSceneModel("library", {
  id: entry.id,
  role: "ground",
  descriptor: `3d-assets/${entry.descriptor}`,
  descriptor_sha256: entry.descriptor_sha256,
  model: `3d-assets/${entry.model}`,
  model_sha256: hash(modelBytes),
  model_scene: entry.model_scene,
  resources: descriptor.resources ?? [],
});
// Terrain is a bare Z-up root, unlike standalone assets' Y-up map wrapper.
assert.deepEqual(
  model
    .getRoot()
    .getDefaultScene()
    .listChildren()
    .map((node) => node.getName()),
  ["ground"],
);
const triangles = maskRecoveryMesh(model, "ground", (p) =>
  sceneToGame({ kind: "oblique-orthographic", elevation_deg: 35 }, p),
);
const meshBounds = [0, 1, 2].map((axis) =>
  triangles
    .flat()
    .reduce(
      (bounds, p) => [Math.min(bounds[0], p[axis]), Math.max(bounds[1], p[axis])],
      [Infinity, -Infinity],
    ),
);
function height(point, triangle) {
  const [a, b, c] = triangle;
  const det = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
  if (Math.abs(det) < 1e-8) return undefined;
  const u = ((b[1] - c[1]) * (point[0] - c[0]) + (c[0] - b[0]) * (point[1] - c[1])) / det;
  const v = ((c[1] - a[1]) * (point[0] - c[0]) + (a[0] - c[0]) * (point[1] - c[1])) / det;
  return u >= -1e-8 && v >= -1e-8 && u + v <= 1 + 1e-8
    ? u * a[2] + v * b[2] + (1 - u - v) * c[2]
    : undefined;
}
const samples = [];
for (let along = 0; along <= 40; along++)
  for (let across = 0; across <= 4; across++) {
    const p = before[0].map((v, i) => v + ((before[1][i] - v) * along) / 40);
    const q = after[0].map((v, i) => v + ((after[1][i] - v) * along) / 40);
    const point = p.map((v, i) => v + ((q[i] - v) * across) / 4);
    const hits = triangles
      .map((triangle) => height(point, triangle))
      .filter((z) => z !== undefined);
    samples.push({ point, hits });
  }
const output = await fs.mkdtemp("work/map-compile/derby-stair-ground-contact-");
const supported = samples.every((sample) => sample.hits.some((z) => Math.abs(z) < 0.05));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify(
    {
      scope: "unpublished mesh-supported terrain contact",
      compiledFile,
      compiledSha256: hash(bytes),
      descriptorSha256: entry.descriptor_sha256,
      modelSha256: hash(modelBytes),
      meshBounds,
      before,
      after,
      supported,
      samples,
    },
    null,
    2,
  ),
);
assert.ok(supported, `Terrain mesh does not support the contact; see ${output}`);
positions.forEach((position, i) => (surface.polygon[position] = after[i]));
validateAssetGameplay(gameplay, descriptor);
edits.push({ asset: entry.id, descriptorSha256: entry.descriptor_sha256, gameplay });
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
console.log(JSON.stringify({ output, supported, samples: samples.length, before, after }));
