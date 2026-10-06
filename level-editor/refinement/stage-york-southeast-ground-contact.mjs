import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh } from "../pipeline/src/mask-recovery-mesh.ts";
import { gltfToScene, sceneToGame } from "../shared/src/geometry.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { compileMap } from "../app/src/map-compile.ts";

// Author the contact in its terrain owner; runtime placement still resolves
// the stair against whatever receiving geometry is present in the editor.
const [stage] = process.argv.slice(2);
assert.ok(stage);
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
const contacts = {
  "york-north-garden-wall-and-stair": {
    lift: "building-049-lift",
    surface: "ground-section-4-0",
    label: "North garden",
    prefix: "york-garden-ground-contact",
    before: [
      [3049, 292],
      [3078, 278],
    ],
  },
  "york-outer-southeast-wall-stair": {
    lift: "building-234-lift",
    label: "Southeast",
    prefix: "york-southeast-ground-contact",
    before: [
      [2466, 2284],
      [2525, 2297],
    ],
  },
  "york-outer-east-upper-wall-stair": {
    lift: "building-242-lift",
    label: "Upper east",
    prefix: "york-east-upper-ground-contact",
    before: [
      [2725, 1911],
      [2761, 1918],
    ],
  },
};
const contact = contacts[edits[0].asset];
assert.ok(contact, "Unknown reviewed terrain contact");
assert.ok(!edits.some((edit) => edit.asset === "york-terrain"));
const document = await readStoredMap("library/scenes/york.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
const group = document.groups.find((group) => group.id === edits[0].asset);
assert.equal(group.transform.rot_deg, 0);
const stair = edits[0].gameplay;
const lift = stair.lifts.find((lift) => lift.id === contact.lift);
const floor = stair.surfaces.find((surface) => surface.id === lift.surface);
const plane = heightPlane(
  floor.polygon.map(([x, y], i) => [
    x + group.transform.dx,
    y + group.transform.dy,
    floor.height[i] + group.transform.dz,
  ]),
);
assert.equal(lift.doors[0].outside[2] + group.transform.dz, 0);
const descriptor = assets.get("york-terrain");
const gameplay = structuredClone(descriptor.gameplay);
const surface = gameplay.surfaces.find(
  (surface) => surface.id === (contact.surface ?? "ground-section-2-0"),
);
assert.ok(surface.height.every((height) => height === 0));
const before = contact.before;
const indices = before.map((p) => surface.polygon.findIndex((q) => p.every((v, i) => v === q[i])));
assert.ok(indices.every((i) => i >= 0));
assert.equal((indices[0] + 1) % surface.polygon.length, indices[1]);
const after = before.map(([x, y]) => {
  const t = planeHeight(plane, [x, y]) / (plane[0] ** 2 + plane[1] ** 2);
  assert.ok(
    Math.abs(t) * Math.hypot(plane[0], plane[1]) < 3,
    "Review larger terrain contact shifts",
  );
  return [x - t * plane[0], y - t * plane[1]];
});
const source = document.sceneAssets.find((source) => source.id === descriptor.id);
assert.ok(source && source.role === "ground");
const model = await loadSceneModel("library", source);
// This ground asset is a camera-facing backdrop, not a 3D terrain mesh. Check
// artwork coverage in map coordinates; this does not certify physical support.
const triangles = maskRecoveryMesh(model, "ground", (point) => {
  const [x, y, z] = sceneToGame(document.camera, gltfToScene(point));
  assert.ok(Math.abs(y) < 0.001, "Review changed ground backdrop geometry");
  return [x, y - z, 0];
});
function meshHeight([x, y], [a, b, c]) {
  const determinant = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
  if (Math.abs(determinant) < 1e-8) return undefined;
  const u = ((b[1] - c[1]) * (x - c[0]) + (c[0] - b[0]) * (y - c[1])) / determinant;
  const v = ((c[1] - a[1]) * (x - c[0]) + (a[0] - c[0]) * (y - c[1])) / determinant;
  return u >= -1e-8 && v >= -1e-8 && u + v <= 1 + 1e-8
    ? u * a[2] + v * b[2] + (1 - u - v) * c[2]
    : undefined;
}
const samples = [];
for (let along = 0; along <= 40; along++)
  for (let across = 0; across <= 4; across++) {
    const point = [0, 1].map((axis) => {
      const p = before[0][axis] + ((before[1][axis] - before[0][axis]) * along) / 40;
      const q = after[0][axis] + ((after[1][axis] - after[0][axis]) * along) / 40;
      return p + ((q - p) * across) / 4;
    });
    const hits = triangles
      .map((triangle) => meshHeight(point, triangle))
      .filter((z) => z !== undefined);
    samples.push({ point, hits, covered: hits.some((z) => Math.abs(z) < 0.001) });
  }
const output = await fs.mkdtemp(`work/map-compile/${contact.prefix}-`);
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({
    stage,
    before,
    after,
    coverageKind: "projected-backdrop-only-not-physical-mesh",
    modelSha256: source.model_sha256,
    samples,
  }),
);
assert.ok(
  samples.every((sample) => sample.covered),
  `Ground artwork does not cover contact: ${output}`,
);
indices.forEach((index, i) => {
  surface.polygon[index] = after[i];
});
surface.preserveMovementPrecision = true;
gameplay.draft.issues.push(
  `${contact.label} stair ground boundary is aligned to the mesh-reviewed flight within a three-unit authoring bound. The ground asset supplies backdrop coverage, not a physical terrain mesh; rendered contact remains unverified.`,
);
validateAssetGameplay(gameplay, descriptor);
edits.push({ asset: descriptor.id, descriptorSha256: source.descriptor_sha256, gameplay });
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
for (const edit of edits) assets.get(edit.asset).gameplay = edit.gameplay;
const compiled = compileMap(document, [0, 0, ...document.size], assets, { bestEffort: true });
await fs.writeFile(`${output}/york.level.json`, JSON.stringify(compiled.descriptor));
await fs.writeFile(
  `${output}/diagnostics.json`,
  JSON.stringify({
    scope: "static-geometry-only-not-gameplay-parity",
    complete: true,
    results: [{ map: "york", file: "york.level.json", warnings: compiled.warnings }],
  }),
);
console.log(JSON.stringify({ output, before, after, samples: samples.length }));
