import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { gameToScene } from "../shared/src/scene.ts";
import { sceneToGame, gltfToScene } from "../shared/src/geometry.ts";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh } from "../pipeline/src/mask-recovery-mesh.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const [stage, compiledFile, marginText = "0", mode] = process.argv.slice(2);
assert.ok(mode === undefined || mode === "--physical-floor");
assert.ok(stage && compiledFile);
const margin = Number(marginText);
assert.ok(Number.isFinite(margin) && margin >= 0);
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8")).filter(
  (edit) => mode !== "--physical-floor" || edit.asset !== "york-terrain",
);
assert.equal(edits[0].asset, "york-east-riverside-southern-wall-stair");
const document = await readStoredMap("library/scenes/york.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
const id = "york-southeast-riverside-raised-terrace",
  node = "building-093";
assert.ok(!edits.some((e) => e.asset === id));
const descriptor = assets.get(id),
  gameplay = structuredClone(descriptor.gameplay);
const part = descriptor.parts.find((p) => p.node === node);
assert.equal(descriptor.parts.length, 1);
assert.equal(gameplay.volumes, undefined);
const shape = structuredClone(part.obstacle_local_game);
const object = document.objects.find((o) => o.node === `asset:${id}:${node}`);
const matrix = partMatrix(document.camera, document, object);
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
for (let i = 0; i < 3; i++) {
  const p = [0, 0, 0];
  p[i] = 1;
  assert.ok(transform(p).every((v, j) => Math.abs(v - origin[j] - (i === j ? 1 : 0)) < 1e-7));
}
const compiled = JSON.parse(await fs.readFile(compiledFile, "utf8")).asset_geometry;
const lifts = compiled.lifts.filter((l) =>
  l.physical_navigation?.doors.some(
    (d) =>
      Math.hypot(d.outside[0] - 2881, d.outside[1] - 1401.001003, d.outside[2] - 50.001003) < 1e-5,
  ),
);
assert.equal(lifts.length, 1);
const [a, b, c] = lifts[0].physical_navigation.plane;
const before = [20, 21].map((i) => structuredClone(shape.points[i]));
for (const i of [20, 21]) {
  const p = shape.points[i],
    world = transform([p.x, p.y, p.z_top]);
  const t = (a * world[0] + b * world[1] + c - world[2]) / (a * a + b * b);
  assert.ok(Math.abs(t) * Math.hypot(a, b) < 0.5);
  p.x -= a * t;
  p.y -= b * t;
}
const after = [20, 21].map((i) => shape.points[i]);
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
const entry = index.find((e) => e.id === id);
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const modelSha256 = hash(await fs.readFile(`library/3d-assets/${entry.model}`));
const model = await loadSceneModel("library", {
  id,
  role: "objects",
  model: `3d-assets/${entry.model}`,
  model_sha256: modelSha256,
  descriptor: `3d-assets/${entry.descriptor}`,
  descriptor_sha256: entry.descriptor_sha256,
  resources: descriptor.resources ?? [],
});
const triangles = maskRecoveryMesh(model, node, (p) =>
  sceneToGame(document.camera, gltfToScene(p)),
);
function height(p, [a, b, c]) {
  const det = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
  if (Math.abs(det) < 1e-8) return undefined;
  const u = ((b[1] - c[1]) * (p[0] - c[0]) + (c[0] - b[0]) * (p[1] - c[1])) / det;
  const v = ((c[1] - a[1]) * (p[0] - c[0]) + (a[0] - c[0]) * (p[1] - c[1])) / det;
  return u >= -1e-8 && v >= -1e-8 && u + v <= 1 + 1e-8
    ? u * a[2] + v * b[2] + (1 - u - v) * c[2]
    : undefined;
}
function distance(p, a, b) {
  const d = [b[0] - a[0], b[1] - a[1]],
    length = d[0] ** 2 + d[1] ** 2;
  const t = Math.max(0, Math.min(1, ((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / (length || 1)));
  return Math.hypot(p[0] - a[0] - t * d[0], p[1] - a[1] - t * d[1]);
}
const top = triangles.filter((t) => t.every((p) => Math.abs(p[2] - before[0].z_top) < 0.1));
assert.ok(top.length);
const samples = [];
for (let i = 0; i <= 40; i++)
  for (let j = 0; j <= 4; j++) {
    const point = ["x", "y"].map((key) => {
      const p = before[0][key] + ((before[1][key] - before[0][key]) * i) / 40;
      const q = after[0][key] + ((after[1][key] - after[0][key]) * i) / 40;
      return p + ((q - p) * j) / 4;
    });
    const supported = top.some((t) => height(point, t) !== undefined);
    const gap = supported
      ? 0
      : Math.min(...top.flatMap((t) => t.map((a, k) => distance(point, a, t[(k + 1) % 3]))));
    samples.push({ point, supported, gap });
  }
const output = await fs.mkdtemp("work/map-compile/york-terrace-receiver-");
const gap = Math.max(...samples.map((s) => s.gap));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({ stage, compiledFile, before, after, modelSha256, samples, gap }),
);
assert.ok(gap <= margin, `Receiver mesh gap ${gap}; see ${output}`);
delete shape.projection_area;
delete shape.material_indices;
const volume = `${node}-reviewed-receiver`;
gameplay.collision = "none";
gameplay.volumes = [{ id: volume, node, shape }];
gameplay.sightOrder = { [volume]: gameplay.sightOrder[node] };
gameplay.projectionReceivers.forEach((r) => {
  assert.equal(r.volume, node);
  r.volume = volume;
});
gameplay.materials.forEach((m) => {
  m.obstacles = m.obstacles.map((v) => {
    assert.equal(v, node);
    return volume;
  });
});
if (mode === "--physical-floor") {
  gameplay.projectionReceivers = [];
  gameplay.movementSolids = [volume];
  gameplay.surfaces = [
    {
      id: `${node}-physical-walkway`,
      node,
      projectionVolume: volume,
      polygon: shape.points.map((p) => [p.x, p.y]),
      height: shape.points.map((p) => p.z_top),
      holes: [],
      preserveMovementPrecision: true,
      preserveMovementBoundary: true,
      navigationRegion: `${node}-physical-walkway`,
    },
  ];
}
gameplay.draft.issues.push(
  `Reviewed terrace stair contact has mesh edge discrepancies up to ${gap.toFixed(3)} game units; rendered actor integration remains unverified.`,
);
validateAssetGameplay(gameplay, descriptor);
edits.push({ asset: id, descriptorSha256: entry.descriptor_sha256, gameplay });
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
console.log(
  JSON.stringify({
    output,
    gap,
    supported: samples.filter((s) => s.supported).length,
    samples: samples.length,
  }),
);
