import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh } from "../pipeline/src/mask-recovery-mesh.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { gameToScene } from "../shared/src/scene.ts";
import { gltfToScene, sceneToGame } from "../shared/src/geometry.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

// Review the ground contact in its terrain and plateau owners. Compiling a new
// placement still requires real matching landings; no scene-specific fix runs.
const [stage, compiledFile] = process.argv.slice(2);
assert.ok(stage && compiledFile, "Provide reviewed stair edits and their Lincoln export");
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
assert.equal(edits.length, 1);
const contacts = {
  "lincoln-south-wall-stair": {
    outside: [1756, 1971.001, 220.001],
    edge: [
      [1758, 1740],
      [1791, 1756],
    ],
    surface: "ground-section-1-0",
    plateau: "lincoln-castle-hill-inner-bailey-plateau",
    node: "building-062",
    blocker: "building-062-ground-blocker-3-0",
    shiftLimit: 2.37,
    meshMargin: 0.375,
    issue:
      "South stair terrain contact has 201/205 exact mesh sample hits; four edge samples extend at most 0.373 units beyond the visible plateau. Complete rendered actor integration remains unverified.",
  },
  "lincoln-east-curtain-wall-middle": {
    outside: [2689, 1213.001, 220.001],
    edge: [
      [2691, 981],
      [2717, 999],
    ],
    surface: "ground-section-2-0",
    plateau: "lincoln-castle-hill-north-bailey-plateau",
    node: "building-067",
    blocker: "building-067-ground-blocker-5-12",
    shiftLimit: 0.79,
    meshMargin: 0.416,
    issue:
      "East curtain stair terrain contact has 200/205 exact mesh sample hits. The end-strip discrepancy decreases from 0.416 units at the existing edge to 0.040 at the corrected edge. Complete rendered actor integration remains unverified.",
  },
};
const contact = contacts[edits[0].asset];
assert.ok(contact, "No reviewed contact for this stair asset");
const document = await readStoredMap("library/scenes/lincoln.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
assert.equal(
  edits[0].descriptorSha256,
  index.find((entry) => entry.id === edits[0].asset).descriptor_sha256,
);
const compiled = JSON.parse(await fs.readFile(compiledFile, "utf8")).asset_geometry;
const lifts = compiled.lifts.filter((lift) =>
  lift.physical_navigation?.doors.some(
    (door) => Math.hypot(...door.outside.map((v, i) => v - contact.outside[i])) < 1e-5,
  ),
);
assert.equal(lifts.length, 1);
const [a, b, c] = lifts[0].physical_navigation.plane;
const z = contact.outside[2];
const before = contact.edge;
const after = before.map(([x, y]) => {
  const t = (a * x + b * (y + z) + c - z) / (a * a + b * b);
  assert.ok(Math.abs(t) * Math.hypot(a, b) < contact.shiftLimit, "Contact exceeds reviewed shift");
  return [x - t * a, y - t * b];
});
const transform = (id, node, point) => {
  const part = document.objects.find((object) => object.node === `asset:${id}:${node}`);
  assert.ok(part);
  const matrix = partMatrix(document.camera, document, part);
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
const plateau = assets.get(contact.plateau);
const entry = index.find((entry) => entry.id === plateau.id);
const modelBytes = await fs.readFile(`library/3d-assets/${entry.model}`);
const model = await loadSceneModel("library", {
  id: entry.id,
  role: "objects",
  descriptor: `3d-assets/${entry.descriptor}`,
  descriptor_sha256: entry.descriptor_sha256,
  model: `3d-assets/${entry.model}`,
  model_sha256: hash(modelBytes),
  resources: plateau.resources ?? [],
  ...(entry.model_scene ? { model_scene: entry.model_scene } : {}),
});
const triangles = maskRecoveryMesh(model, contact.node, (p) =>
  transform(plateau.id, contact.node, sceneToGame(document.camera, gltfToScene(p))),
);
function meshHeight(point, [a, b, c]) {
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
    const projected = p.map((v, i) => v + ((q[i] - v) * across) / 4);
    const world = [projected[0], projected[1] + z, z];
    const hits = triangles.map((t) => meshHeight(world, t)).filter((h) => h !== undefined);
    const edgeDistance = (a, b) => {
      const dx = b[0] - a[0],
        dy = b[1] - a[1];
      const t = Math.max(
        0,
        Math.min(1, ((world[0] - a[0]) * dx + (world[1] - a[1]) * dy) / (dx * dx + dy * dy || 1)),
      );
      return Math.hypot(world[0] - a[0] - t * dx, world[1] - a[1] - t * dy);
    };
    const uncoveredDistance = hits.some((h) => Math.abs(h - z) < 0.06)
      ? 0
      : Math.min(
          ...triangles
            .filter((t) => t.every((p) => Math.abs(p[2] - z) < 0.06))
            .flatMap((t) => t.map((p, i) => edgeDistance(p, t[(i + 1) % 3]))),
        );
    samples.push({ world, hits, uncoveredDistance });
  }
// Explicit per-contact authoring bounds do not relax runtime floor matching.
const maximumUncoveredDistance = Math.max(...samples.map((s) => s.uncoveredDistance));
const supported = maximumUncoveredDistance <= contact.meshMargin;
const output = await fs.mkdtemp("work/map-compile/lincoln-stair-ground-contact-");
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify(
    {
      before,
      after,
      supported,
      samples,
      maximumUncoveredDistance,
      modelSha256: hash(modelBytes),
      scope: "mesh-reviewed terrain and plateau contact; not rendered actor verification",
    },
    null,
    2,
  ),
);
assert.ok(supported, `Plateau mesh does not support contact: ${output}`);
const terrain = assets.get("lincoln-terrain");
const terrainGameplay = structuredClone(terrain.gameplay);
const surface = terrainGameplay.surfaces.find((s) => s.id === contact.surface);
assert.ok(surface.height.every((h) => h === 0));
const positions = before.map((p) =>
  surface.polygon.findIndex((q) => p.every((v, i) => v === q[i])),
);
assert.ok(positions.every((i) => i >= 0));
assert.equal((positions[0] + 1) % surface.polygon.length, positions[1]);
positions.forEach((i, j) => (surface.polygon[i] = after[j]));
surface.preserveMovementPrecision = true;
const plateauGameplay = structuredClone(plateau.gameplay);
if (contact.issue) {
  assert.ok(plateauGameplay.draft?.issues, "Expected reviewed draft plateau");
  if (!plateauGameplay.draft.issues.includes(contact.issue))
    plateauGameplay.draft.issues.push(contact.issue);
}
const blocker = plateauGameplay.movementBlockers.find((b) => b.id === contact.blocker);
const origin = transform(plateau.id, blocker.node, [0, 0, 0]);
for (const axis of [0, 1]) {
  const point = [0, 0, 0];
  point[axis] = 1;
  assert.ok(
    transform(plateau.id, blocker.node, point).every(
      (v, i) => Math.abs(v - origin[i] - (axis === i ? 1 : 0)) < 1e-7,
    ),
  );
}
const blockerPositions = before.map((p) =>
  blocker.polygon.findIndex((local) => {
    const world = transform(plateau.id, blocker.node, [...local, 0]);
    return Math.hypot(world[0] - p[0], world[1] - world[2] - p[1]) < 1e-6;
  }),
);
assert.ok(blockerPositions.every((i) => i >= 0));
assert.equal((blockerPositions[1] + 1) % blocker.polygon.length, blockerPositions[0]);
blockerPositions.forEach((position, i) => {
  blocker.polygon[position] = blocker.polygon[position].map(
    (v, axis) => v + after[i][axis] - before[i][axis],
  );
});
blocker.preserveMovementPrecision = true;
for (const [descriptor, gameplay] of [
  [terrain, terrainGameplay],
  [plateau, plateauGameplay],
]) {
  validateAssetGameplay(gameplay, descriptor);
  edits.push({
    asset: descriptor.id,
    descriptorSha256: index.find((e) => e.id === descriptor.id).descriptor_sha256,
    gameplay,
  });
}
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits, null, 2));
console.log(
  JSON.stringify({
    output,
    reviewedSamples: samples.length,
    exactMeshHits: samples.filter((s) => s.uncoveredDistance === 0).length,
    maximumUncoveredDistance,
    assets: edits.map((e) => e.asset),
  }),
);
