import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh } from "../pipeline/src/mask-recovery-mesh.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { gameToScene } from "../shared/src/scene.ts";
import { gltfToScene, sceneToGame } from "../shared/src/geometry.ts";
import { planeHeight } from "../shared/src/gameplay-plane.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

// Stage the receiving asset's own access opening. Publication requires mesh
// review and traversal with independently placed copies of both assets.
const [stage, compiledFile] = process.argv.slice(2);
assert.ok(stage && compiledFile);
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
assert.equal(edits.length, 1);
assert.equal(edits[0].asset, "nottingham-castle-west-stair-tower");
const document = await readStoredMap("library/scenes/nottingham.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
const stairSource = document.assetSources.find((s) => s.id === edits[0].asset);
assert.equal(stairSource.descriptor_sha256, edits[0].descriptorSha256);
const flightReviews = JSON.parse(await fs.readFile(`${stage}/mesh-review.json`, "utf8"));
assert.equal(flightReviews.length, 1);
const flight = flightReviews[0];
assert.equal(flight.asset, edits[0].asset);
assert.equal(flight.modelSha256, stairSource.model_sha256);
edits[0].gameplay.draft.issues.push(
  `West stair tower has ${flight.sampledMeshHits}/${flight.sampledFloorPoints} mesh samples supported with gaps up to ${flight.maximumUncoveredMeshEdgeDistance.toFixed(3)} game units; some gaps are visible after rotation and rendered actor integration remains unverified.`,
);
const asset = "nottingham-castle-hall-and-watchtower";
const descriptor = assets.get(asset);
const source = document.assetSources.find((s) => s.id === asset);
assert.equal(
  createHash("sha256")
    .update(await fs.readFile(`library/${source.descriptor}`))
    .digest("hex"),
  source.descriptor_sha256,
);
const gameplay = structuredClone(descriptor.gameplay);
const surface = gameplay.surfaces.find((s) => s.id === "building-488-walk-0");
const part = document.objects.find((o) => o.node === `asset:${asset}:${surface.node}`);
const matrix = partMatrix(document.camera, document, part);
const transform = (point) => {
  const p = gameToScene(document.camera, ...point);
  return sceneToGame(
    document.camera,
    [0, 1, 2].map(
      (r) => matrix[r] * p[0] + matrix[r + 4] * p[1] + matrix[r + 8] * p[2] + matrix[r + 12],
    ),
  );
};
const origin = transform([0, 0, 0]);
for (const axis of [0, 1, 2]) {
  const p = [0, 0, 0];
  p[axis] = 1;
  assert.ok(transform(p).every((v, i) => Math.abs(v - origin[i] - (i === axis ? 1 : 0)) < 1e-7));
}
const compiled = JSON.parse(await fs.readFile(compiledFile, "utf8")).asset_geometry;
const candidates = compiled.lifts.filter((l) =>
  l.physical_navigation?.doors.some(
    (d) => Math.hypot(...d.outside.map((v, i) => v - [263, 1274.00104, 480.00104][i])) < 1e-4,
  ),
);
assert.equal(candidates.length, 1);
const floor = candidates[0].physical_navigation;
const height = transform([...surface.polygon[0], surface.height[0]])[2];
const seam = floor.boundary.filter((p) => Math.abs(planeHeight(floor.plane, p) - height) < 1e-4);
assert.equal(seam.length, 3);
seam.sort((a, b) => a[0] - b[0]);
const after = [seam.at(-1), seam[0]].map((p) => [...p, height]);
const indices = [15, 16];
const before = indices.map((i) => transform([...surface.polygon[i], surface.height[i]]));
assert.ok(
  before.every(
    (p, i) =>
      Math.hypot(
        ...p.map(
          (v, j) =>
            v -
            [
              [278, 1292.00104, height],
              [237, 1279.00104, height],
            ][i][j],
        ),
      ) < 1e-5,
  ),
);
const shifts = after.map((p, i) => Math.hypot(...p.map((v, j) => v - before[i][j])));
assert.ok(
  shifts.every((shift) => shift < 1.5),
  "Receiving edge correction needs broader review",
);
const model = await loadSceneModel("library", source);
const triangles = maskRecoveryMesh(model, surface.node, (p) =>
  transform(sceneToGame(document.camera, gltfToScene(p))),
);
const floorTriangles = triangles.filter((t) => t.every((p) => Math.abs(p[2] - height) < 0.25));
function meshHeight(p, [a, b, c]) {
  const det = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
  if (Math.abs(det) < 1e-8) return undefined;
  const u = ((b[1] - c[1]) * (p[0] - c[0]) + (c[0] - b[0]) * (p[1] - c[1])) / det;
  const v = ((c[1] - a[1]) * (p[0] - c[0]) + (a[0] - c[0]) * (p[1] - c[1])) / det;
  return u >= -1e-8 && v >= -1e-8 && u + v <= 1 + 1e-8
    ? u * a[2] + v * b[2] + (1 - u - v) * c[2]
    : undefined;
}
function edgeDistance(p, a, b) {
  const d = b.map((v, i) => v - a[i]);
  const t = Math.max(
    0,
    Math.min(
      1,
      d.reduce((sum, v, i) => sum + v * (p[i] - a[i]), 0) /
        (d.reduce((sum, v) => sum + v * v, 0) || 1),
    ),
  );
  return Math.hypot(...p.map((v, i) => v - a[i] - t * d[i]));
}
const samples = [];
for (let along = 0; along <= 40; along++)
  for (let across = 0; across <= 4; across++) {
    const p = before[0].map((v, i) => v + ((before[1][i] - v) * along) / 40);
    const q = after[0].map((v, i) => v + ((after[1][i] - v) * along) / 40);
    const point = p.map((v, i) => v + ((q[i] - v) * across) / 4);
    const supported = triangles.some((t) => {
      const z = meshHeight(point, t);
      return z !== undefined && Math.abs(z - point[2]) < 0.1;
    });
    const gap = supported
      ? 0
      : Math.min(
          ...floorTriangles.flatMap((t) => t.map((a, i) => edgeDistance(point, a, t[(i + 1) % 3]))),
        );
    samples.push({ point, supported, gap });
  }
const supported = samples.filter((s) => s.supported).length;
const gap = Math.max(...samples.map((s) => s.gap));
assert.ok(Number.isFinite(gap), "Receiving contact has no reviewable floor mesh");
indices.forEach((index, i) => {
  surface.polygon[index] = after[i].slice(0, 2).map((v, j) => v - origin[j]);
  surface.height[index] = after[i][2] - origin[2];
});
surface.preserveMovementPrecision = true;
const clearance = {
  id: "building-488-west-stair-access-clearance",
  node: surface.node,
  polygon: floor.boundary.map((p) => p.map((v, i) => v - origin[i])),
  height: floor.boundary.map((p) => planeHeight(floor.plane, p) - origin[2]),
  holes: [],
};
assert.ok(!gameplay.movementClearances.some((c) => c.id === clearance.id));
gameplay.movementClearances.push(clearance);
gameplay.draft.issues.push(
  `West stair access contact has ${supported}/${samples.length} mesh samples supported with gaps up to ${gap.toFixed(3)} game units; rendered actor integration remains unverified.`,
);
validateAssetGameplay(gameplay, descriptor);
edits.push({ asset, descriptorSha256: source.descriptor_sha256, gameplay });
const output = await fs.mkdtemp("work/map-compile/nottingham-west-stair-contact-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify(
    {
      scope: "unpublished receiver and asset-owned access clearance; mesh review required",
      stage,
      compiledFile,
      asset,
      surface: surface.id,
      before,
      after,
      shifts,
      clearance,
      modelSha256: source.model_sha256,
      samples,
      supported,
      gap,
    },
    null,
    2,
  ),
);
console.log(
  JSON.stringify({ output, before, after, shifts, supported, samples: samples.length, gap }),
);
