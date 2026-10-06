import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh } from "../pipeline/src/mask-recovery-mesh.ts";
import { sceneToGame } from "../shared/src/geometry.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

// Stage the terrain-owned strip at a reviewed stair contact. The compiler still
// derives connectivity from placed geometry and does not fill unsupported gaps.
const [stage, compiledFile] = process.argv.slice(2);
assert.ok(stage && compiledFile, "Provide stair edits and their Nottingham export");
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
assert.equal(edits.length, 1);
const contacts = {
  "nottingham-south-stair-house": {
    outside: [1688, 2004, 0],
    surface: "ground-section-0-0",
    edge: [
      [1678, 1921],
      [1704, 2016],
    ],
    partial: true,
    landingMargin: 0.31,
    issue:
      "South stair flight has complete sampled mesh coverage; corrected upper landing edges extend at most 0.310 game units beyond their mesh. Complete rendered actor integration remains unverified.",
  },
  "nottingham-southwest-prison": {
    outside: [355, 2059, 0],
    surface: "ground-section-6-0",
    edge: [
      [351, 2040],
      [388, 2074],
    ],
    partial: false,
    landingMargin: 0.03,
    issue:
      "Prison stair flight has complete sampled mesh coverage; corrected upper landing edges extend at most 0.029 game units beyond their mesh. Complete rendered actor integration remains unverified.",
  },
};
const contact = contacts[edits[0].asset];
assert.ok(contact, "No reviewed terrain contact for this stair asset");
const compiledBytes = await fs.readFile(compiledFile);
const compiled = JSON.parse(compiledBytes).asset_geometry;
const matches = compiled.lifts.filter((lift) =>
  lift.physical_navigation?.doors.some(
    (door) => Math.hypot(...door.outside.map((v, i) => v - contact.outside[i])) < 1e-5,
  ),
);
assert.equal(matches.length, 1);
const stair = matches[0].physical_navigation;
const [a, b, c] = stair.plane;
const length = Math.hypot(a, b);
const sideways = ([x, y]) => (-b * x + a * y) / length;
const seam = stair.boundary.filter(([x, y]) => Math.abs(a * x + b * y + c) < 1e-5);
assert.equal(seam.length, 2);
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
assert.equal(
  index.find((e) => e.id === edits[0].asset).descriptor_sha256,
  edits[0].descriptorSha256,
);
const entry = index.find((e) => e.id === "nottingham-terrain");
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
assert.equal(hash(bytes), entry.descriptor_sha256);
const descriptor = JSON.parse(bytes);
const gameplay = structuredClone(descriptor.gameplay);
const surface = gameplay.surfaces.find((s) => s.id === contact.surface);
assert.ok(surface.height.every((z) => z === 0));
const edge = contact.edge;
const position = surface.polygon.findIndex((p) => p.every((v, i) => v === edge[0][i]));
assert.ok(position >= 0);
assert.deepEqual(surface.polygon[position + 1], edge[1]);
const motion = compiled.motion_data.layers[matches[0].doors[0].layer_out];
assert.ok(
  motion.some((region) =>
    edge.every((p) => region.polygon.points.some((q) => p.every((v, i) => v === q[i]))),
  ),
);
const start = sideways(edge[0]),
  end = sideways(edge[1]);
const parameters = contact.partial
  ? seam.map((p) => (sideways(p) - start) / (end - start)).sort((x, y) => x - y)
  : [0, 1];
assert.ok(parameters.every((t) => t >= 0 && t <= 1));
const before = parameters.map((t) => edge[0].map((v, i) => v + t * (edge[1][i] - v)));
const after = before.map(([x, y]) => {
  const t = (a * x + b * y + c) / (a * a + b * b);
  assert.ok(Math.abs(t) * length < 2.01, "Terrain contact exceeds the reviewed 2.01-unit strip");
  return [x - a * t, y - b * t];
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
assert.deepEqual(
  model
    .getRoot()
    .getDefaultScene()
    .listChildren()
    .map((n) => n.getName()),
  ["ground"],
);
const triangles = maskRecoveryMesh(model, "ground", (p) =>
  sceneToGame({ kind: "oblique-orthographic", elevation_deg: 35 }, p),
);
function height(point, [a, b, c]) {
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
    const hits = triangles.map((t) => height(point, t)).filter((z) => z !== undefined);
    samples.push({ point, hits });
  }
const supported = samples.every((s) => s.hits.some((z) => Math.abs(z) < 0.05));
const output = await fs.mkdtemp(`work/map-compile/${edits[0].asset}-contact-`);
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify(
    {
      compiledFile,
      compiledSha256: hash(compiledBytes),
      descriptorSha256: hash(bytes),
      modelSha256: hash(modelBytes),
      before,
      after,
      supported,
      samples,
    },
    null,
    2,
  ),
);
assert.ok(supported, `Terrain mesh does not support contact; see ${output}`);
const meshReviews = JSON.parse(await fs.readFile(`${stage}/mesh-review.json`, "utf8"));
assert.equal(meshReviews.length, 1);
const meshReview = meshReviews[0];
assert.equal(meshReview.asset, edits[0].asset);
assert.equal(meshReview.sampledMeshHits, meshReview.sampledFloorPoints);
assert.ok(
  meshReview.landingEdgeReviews.every(
    (edge) => edge.maximumUncoveredDistance < contact.landingMargin,
  ),
);
const stairEntry = index.find((e) => e.id === edits[0].asset);
assert.equal(
  hash(await fs.readFile(`library/3d-assets/${stairEntry.model}`)),
  meshReview.modelSha256,
);
edits[0].gameplay.draft ??= { issues: [] };
edits[0].gameplay.draft.issues.push(contact.issue);
if (contact.partial)
  surface.polygon.splice(position + 1, 0, before[0], after[0], after[1], before[1]);
else surface.polygon.splice(position, 2, ...after);
surface.height = surface.polygon.map(() => 0);
surface.preserveMovementPrecision = true;
validateAssetGameplay(gameplay, descriptor);
edits.push({ asset: entry.id, descriptorSha256: entry.descriptor_sha256, gameplay });
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
console.log(JSON.stringify({ output, supported, samples: samples.length, before, after }));
