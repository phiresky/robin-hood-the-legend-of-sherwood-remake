import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh } from "../pipeline/src/mask-recovery-mesh.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { gameToScene } from "../shared/src/scene.ts";
import { gltfToScene, sceneToGame } from "../shared/src/geometry.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

// Stage independently owned receiving edges and access clearances; never publish here.
const [stage, compiledFile, ...flags] = process.argv.slice(2);
assert.ok(stage && compiledFile && flags.every((f) => f.startsWith("--mesh-margin=")));
const margin = Number(flags[0]?.split("=")[1] ?? 0);
assert.ok(Number.isFinite(margin) && margin >= 0 && flags.length <= 1);
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
assert.ok(edits.some((e) => e.asset === "nottingham-castle-west-stair"));
const document = await readStoredMap("library/scenes/nottingham.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
const stairSource = document.assetSources.find((s) => s.id === "nottingham-castle-west-stair");
const stairEdit = edits.find((e) => e.asset === stairSource.id);
assert.equal(stairEdit.descriptorSha256, stairSource.descriptor_sha256);
const flightReviews = JSON.parse(await fs.readFile(`${stage}/mesh-review.json`, "utf8"));
assert.equal(flightReviews.length, 1);
const flight = flightReviews[0];
assert.equal(flight.asset, stairSource.id);
assert.equal(flight.modelSha256, stairSource.model_sha256);
stairEdit.gameplay.draft.issues.push(
  `Courtyard western stair has only ${flight.sampledMeshHits}/${flight.sampledFloorPoints} sampled floor points supported by its mesh, with visible gaps up to ${flight.maximumUncoveredMeshEdgeDistance.toFixed(3)} game units. The shortened mesh flight needs repair; gameplay traversal checks do not establish rendered parity.`,
);
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
const compiled = JSON.parse(await fs.readFile(compiledFile, "utf8")).asset_geometry;
const lifts = compiled.lifts.filter((l) =>
  l.physical_navigation?.doors.some(
    (d) => Math.hypot(...d.outside.map((v, i) => v - [322, 1526.00101, 100.00101][i])) < 1e-4,
  ),
);
assert.equal(lifts.length, 1);
const floor = lifts[0].physical_navigation;
const output = await fs.mkdtemp("work/map-compile/nottingham-castle-west-stair-contacts-");
const changes = [];
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
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
for (const [id, surfaceId, indices] of [
  ["nottingham-castle-courtyard-ground", "building-366-walk-0", [6, 7]],
  ["nottingham-castle-west-courtyard-wall", "building-327-walk-0", [2, 3]],
]) {
  assert.ok(!edits.some((e) => e.asset === id));
  const descriptor = assets.get(id),
    gameplay = structuredClone(descriptor.gameplay);
  const surface = gameplay.surfaces.find((s) => s.id === surfaceId);
  const part = document.objects.find((o) => o.node === `asset:${id}:${surface.node}`);
  assert.ok(part);
  const matrix = partMatrix(document.camera, document, part);
  const transform = (point) => {
    const p = gameToScene(document.camera, ...point);
    return sceneToGame(
      document.camera,
      [0, 1, 2].map(
        (r) => matrix[r] * p[0] + matrix[4 + r] * p[1] + matrix[8 + r] * p[2] + matrix[12 + r],
      ),
    );
  };
  const origin = transform([0, 0, 0]);
  for (const axis of [0, 1, 2]) {
    const p = [0, 0, 0];
    p[axis] = 1;
    assert.ok(
      transform(p).every((v, i) => Math.abs(v - origin[i] - (i === axis ? 1 : 0)) < 1e-7),
      "Expected translation-only authoring frame",
    );
  }
  const points = surface.polygon.map((p, i) => transform([...p, surface.height[i]]));
  const plane = heightPlane(points);
  const seam = floor.boundary
    .filter((p) => Math.abs(planeHeight(floor.plane, p) - planeHeight(plane, p)) < 1e-4)
    .map((p) => [...p, planeHeight(plane, p)]);
  assert.equal(seam.length, 2);
  const before = indices.map((i) => points[i]);
  const score = (ends) =>
    ends.reduce((sum, p, i) => sum + Math.hypot(...p.map((v, j) => v - before[i][j])), 0);
  const after = score(seam) <= score([...seam].reverse()) ? seam : [...seam].reverse();
  const shifts = after.map((p, i) => Math.hypot(...p.map((v, j) => v - before[i][j])));
  assert.ok(
    shifts.every((v) => v < 3.1),
    `${id}: excessive boundary correction`,
  );
  const entry = index.find((e) => e.id === id);
  const modelBytes = await fs.readFile(`library/3d-assets/${entry.model}`);
  const model = await loadSceneModel("library", {
    id,
    role: "objects",
    descriptor: `3d-assets/${entry.descriptor}`,
    descriptor_sha256: entry.descriptor_sha256,
    model: `3d-assets/${entry.model}`,
    model_sha256: hash(modelBytes),
    resources: descriptor.resources ?? [],
    ...(entry.model_scene ? { model_scene: entry.model_scene } : {}),
  });
  const triangles = maskRecoveryMesh(model, surface.node, (p) =>
    transform(sceneToGame(document.camera, gltfToScene(p))),
  );
  const floorTriangles = triangles.filter((triangle) =>
    triangle.every((p) => Math.abs(p[2] - planeHeight(plane, p)) < 0.25),
  );
  const samples = [];
  for (let along = 0; along <= 40; along++)
    for (let across = 0; across <= 4; across++) {
      const p = before[0].map((v, i) => v + ((before[1][i] - v) * along) / 40);
      const q = after[0].map((v, i) => v + ((after[1][i] - v) * along) / 40);
      const point = p.map((v, i) => v + ((q[i] - v) * across) / 4);
      const hits = triangles.map((t) => meshHeight(point, t)).filter((h) => h !== undefined);
      const supported = hits.some((h) => Math.abs(h - point[2]) < 0.1);
      const gap = supported
        ? 0
        : Math.min(
            ...floorTriangles.flatMap((t) =>
              t.map((a, i) => edgeDistance(point, a, t[(i + 1) % 3])),
            ),
          );
      samples.push({ point, hits, supported, gap });
    }
  const supported = samples.filter((s) => s.supported).length;
  const gap = Math.max(...samples.map((s) => s.gap));
  changes.push({
    asset: id,
    surface: surfaceId,
    before,
    after,
    shifts,
    supported,
    sampleCount: samples.length,
    gap,
    samples,
    modelSha256: hash(modelBytes),
  });
  await fs.writeFile(
    `${output}/review.json`,
    JSON.stringify({ input: stage, compiledFile, margin, changes }, null, 2),
  );
  assert.ok(
    Number.isFinite(gap) && gap <= margin,
    `${id}: mesh support ${supported}/${samples.length}, gap ${gap}; review ${output}`,
  );
  indices.forEach((index, i) => {
    surface.polygon[index] = after[i].slice(0, 2).map((v, j) => v - origin[j]);
    surface.height[index] = after[i][2] - origin[2];
  });
  surface.preserveMovementPrecision = true;
  if (id === "nottingham-castle-courtyard-ground") {
    assert.equal(descriptor.parts.length, 1);
    assert.equal(gameplay.collision, "parts");
    assert.equal(gameplay.volumes?.length ?? 0, 0);
    assert.equal(gameplay.movementTransitions.length, 0);
    const part = descriptor.parts[0];
    assert.equal(surface.projectionVolume, part.node);
    const shape = structuredClone(part.obstacle_local_game);
    delete shape.projection_area;
    delete shape.material_indices;
    const physicalIndices = [12, 13];
    const physicalBefore = physicalIndices.map((i) => {
      const p = shape.points[i];
      return transform([p.x, p.y, p.z_top]);
    });
    assert.ok(physicalBefore.every((p, i) => Math.hypot(...p.map((v, j) => v - before[i][j])) < 3));
    const physicalSamples = [];
    for (let along = 0; along <= 40; along++)
      for (let across = 0; across <= 4; across++) {
        const p = physicalBefore[0].map((v, i) => v + ((physicalBefore[1][i] - v) * along) / 40);
        const q = after[0].map((v, i) => v + ((after[1][i] - v) * along) / 40);
        const point = p.map((v, i) => v + ((q[i] - v) * across) / 4);
        const hits = triangles.map((t) => meshHeight(point, t)).filter((h) => h !== undefined);
        const supported = hits.some((h) => Math.abs(h - point[2]) < 0.1);
        const gap = supported
          ? 0
          : Math.min(
              ...floorTriangles.flatMap((t) =>
                t.map((a, i) => edgeDistance(point, a, t[(i + 1) % 3])),
              ),
            );
        physicalSamples.push({ point, hits, supported, gap });
      }
    const physicalGap = Math.max(...physicalSamples.map((s) => s.gap));
    assert.ok(physicalGap <= margin, `Physical receiver mesh gap ${physicalGap}`);
    changes.at(-1).physicalReceiver = {
      before: physicalBefore,
      after,
      samples: physicalSamples,
      gap: physicalGap,
    };
    physicalIndices.forEach((index, i) => {
      shape.points[index].x = after[i][0] - origin[0];
      shape.points[index].y = after[i][1] - origin[1];
    });
    const volume = `${part.node}-reviewed-stair-receiver`;
    gameplay.collision = "none";
    gameplay.volumes = [{ id: volume, node: part.node, shape }];
    surface.projectionVolume = volume;
    for (const material of gameplay.materials)
      material.obstacles = material.obstacles.map((node) => (node === part.node ? volume : node));
    gameplay.sightOrder[volume] = gameplay.sightOrder[part.node];
    delete gameplay.sightOrder[part.node];
  }
  const clearance = {
    id: `${surface.id}-western-stair-access-clearance`,
    node: surface.node,
    polygon: floor.boundary.map((p) => p.map((v, i) => v - origin[i])),
    height: floor.boundary.map((p) => planeHeight(floor.plane, p) - origin[2]),
    holes: [],
  };
  assert.ok(!gameplay.movementClearances.some((c) => c.id === clearance.id));
  gameplay.movementClearances.push(clearance);
  if (gap > 0) {
    gameplay.draft ??= { issues: [] };
    gameplay.draft.issues.push(
      `Courtyard western stair contact has ${supported}/${samples.length} mesh samples supported, with gaps up to ${gap.toFixed(3)} game units; rendered actor integration remains unverified.`,
    );
  }
  validateAssetGameplay(gameplay, descriptor);
  edits.push({ asset: id, descriptorSha256: entry.descriptor_sha256, gameplay });
}
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits, null, 2));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({ input: stage, compiledFile, margin, changes }, null, 2),
);
console.log(
  JSON.stringify({
    output,
    changes: changes.map(({ samples, physicalReceiver, ...change }) => ({
      ...change,
      ...(physicalReceiver
        ? {
            physicalReceiver: {
              before: physicalReceiver.before,
              after: physicalReceiver.after,
              sampleCount: physicalReceiver.samples.length,
              supported: physicalReceiver.samples.filter((s) => s.supported).length,
              gap: physicalReceiver.gap,
            },
          }
        : {}),
    })),
  }),
);
