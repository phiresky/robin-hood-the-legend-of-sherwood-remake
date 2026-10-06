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

// Stage the independently owned receiving edges; never publish from this tool.
const [stage, compiledFile, ...flags] = process.argv.slice(2);
assert.ok(stage && compiledFile && flags.every((f) => f.startsWith("--mesh-margin=")));
const margin = Number(flags[0]?.split("=")[1] ?? 0);
assert.ok(Number.isFinite(margin) && margin >= 0 && flags.length <= 1);
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
assert.ok(edits.some((e) => e.asset === "lincoln-great-hall"));
const document = await readStoredMap("library/scenes/lincoln.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
const compiled = JSON.parse(await fs.readFile(compiledFile, "utf8")).asset_geometry;
const lifts = compiled.lifts.filter((l) =>
  l.physical_navigation?.doors.some(
    (d) =>
      Math.hypot(...d.outside.map((v, i) => v - [1455, 1534.0340746845136, 416.0340746845137][i])) <
      1e-4,
  ),
);
assert.equal(lifts.length, 1);
const floor = lifts[0].physical_navigation;
const output = await fs.mkdtemp("work/map-compile/lincoln-great-hall-contacts-");
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
  ["lincoln-hall-approach-ramp", "building-288-walk-0", [6, 7]],
  ["lincoln-keep-annex", "building-198-walk-0", [23, 0]],
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
    shifts.every((v) => v < 6),
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
  if (gap > 0) {
    gameplay.draft ??= { issues: [] };
    gameplay.draft.issues.push(
      `Great-hall landing contact has ${supported}/${samples.length} mesh samples supported, with gaps up to ${gap.toFixed(3)} game units; rendered actor integration remains unverified.`,
    );
  }
  validateAssetGameplay(gameplay, descriptor);
  edits.push({ asset: id, descriptorSha256: entry.descriptor_sha256, gameplay });
}
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits, null, 2));
console.log(JSON.stringify({ output, changes: changes.map(({ samples, ...change }) => change) }));
