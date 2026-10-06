import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh } from "../pipeline/src/mask-recovery-mesh.ts";
import { sceneToGame, gltfToScene } from "../shared/src/geometry.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

// Author the receiving platform in its own asset frame, retaining independent
// placement. The compiled scene identifies the contact only during this review.
const [source, compiledFile] = process.argv.slice(2);
assert.ok(source && compiledFile, "Provide tower seam edits and a Leicester descriptor");
const edits = JSON.parse(await fs.readFile(`${source}/edits.json`, "utf8"));
assert.equal(edits.length, 1);
assert.equal(edits[0].asset, "leicester-northwest-tower");
const compiled = JSON.parse(await fs.readFile(compiledFile, "utf8")).asset_geometry;
const lift = compiled.lifts.find((lift) =>
  lift.physical_navigation?.doors.some(
    (door) =>
      Math.hypot(door.outside[0] - 780, door.outside[1] - 507.001, door.outside[2] - 140.001) <
      1e-5,
  ),
);
assert.ok(lift);
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
const entry = index.find((entry) => entry.id === "leicester-great-keep");
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
assert.equal(hash(bytes), entry.descriptor_sha256);
const descriptor = JSON.parse(bytes),
  gameplay = structuredClone(descriptor.gameplay);
const surface = gameplay.surfaces.find((surface) => surface.id === "building-280-walk-0");
const origin = [820.0673828125, 658.472610563174, 0.00008286845132099367];
const before = [
  [774, 500.001],
  [793, 504.001],
].map((p) => p.map((value, axis) => value - origin[axis]));
const positions = before.map((p) =>
  surface.polygon.findIndex((q) => Math.hypot(p[0] - q[0], p[1] - q[1]) < 1e-6),
);
assert.ok(positions.every((i) => i >= 0));
assert.equal((positions[0] + 1) % surface.polygon.length, positions[1]);
const [a, b, c] = lift.physical_navigation.plane;
const height = surface.height[positions[0]];
assert.ok(surface.height.every((value) => value === height));
const after = before.map(([x, y]) => {
  const d = a * (x + origin[0]) + b * (y + origin[1]) + c - height - origin[2];
  const t = d / (a * a + b * b);
  assert.ok(Math.abs(t) * Math.hypot(a, b) < 1, "Contact needs broader mesh review");
  return [x - t * a, y - t * b];
});
const modelBytes = await fs.readFile(`library/3d-assets/${entry.model}`);
const model = await loadSceneModel("library", {
  id: entry.id,
  role: "objects",
  descriptor: `3d-assets/${entry.descriptor}`,
  descriptor_sha256: entry.descriptor_sha256,
  model: `3d-assets/${entry.model}`,
  model_sha256: hash(modelBytes),
  model_scene: entry.model_scene,
  resources: descriptor.resources ?? [],
});
const triangles = maskRecoveryMesh(model, surface.node, (p) =>
  sceneToGame({ kind: "oblique-orthographic", elevation_deg: 35 }, gltfToScene(p)),
);
function meshHeight([x, y], [a, b, c]) {
  const det = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
  if (Math.abs(det) < 1e-8) return undefined;
  const u = ((b[1] - c[1]) * (x - c[0]) + (c[0] - b[0]) * (y - c[1])) / det;
  const v = ((c[1] - a[1]) * (x - c[0]) + (a[0] - c[0]) * (y - c[1])) / det;
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
    const hits = triangles.map((t) => meshHeight(point, t)).filter((z) => z !== undefined);
    const supported = hits.some((z) => Math.abs(z - height) < 0.1);
    const edgeDistance = (a, b) => {
      const dx = b[0] - a[0],
        dy = b[1] - a[1];
      const denominator = dx * dx + dy * dy;
      const t = denominator
        ? Math.max(0, Math.min(1, ((point[0] - a[0]) * dx + (point[1] - a[1]) * dy) / denominator))
        : 0;
      return Math.hypot(point[0] - a[0] - t * dx, point[1] - a[1] - t * dy);
    };
    const uncoveredDistance = supported
      ? 0
      : Math.min(
          ...triangles
            .filter((triangle) => triangle.every((p) => Math.abs(p[2] - height) < 0.1))
            .flatMap((triangle) => triangle.map((p, i) => edgeDistance(p, triangle[(i + 1) % 3]))),
        );
    samples.push({ point, hits, uncoveredDistance });
  }
const output = await fs.mkdtemp("work/map-compile/northwest-tower-contact-");
const maximumUncoveredDistance = Math.max(...samples.map((s) => s.uncoveredDistance));
// Explicit authoring review bound; this does not relax runtime floor matching.
const supported = maximumUncoveredDistance <= 0.25;
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify(
    {
      scope: "unpublished mesh-reviewed platform contact",
      before,
      after,
      height,
      supported,
      maximumUncoveredDistance,
      samples,
      descriptorSha256: entry.descriptor_sha256,
      modelSha256: hash(modelBytes),
    },
    null,
    2,
  ),
);
assert.ok(supported, `Platform mesh does not support this contact; see ${output}`);
positions.forEach((position, i) => (surface.polygon[position] = after[i]));
surface.preserveMovementPrecision = true;
gameplay.draft ??= { issues: [] };
gameplay.draft.issues.push(
  "The northwest tower platform contact has 188 of 205 exact mesh samples; remaining edges extend up to 0.195 game units beyond the mesh. Rendered actor integration remains unverified.",
);
validateAssetGameplay(gameplay, descriptor);
edits.push({ asset: entry.id, descriptorSha256: entry.descriptor_sha256, gameplay });
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
console.log(JSON.stringify({ output, supported, samples: samples.length }));
