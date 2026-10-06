import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh } from "../pipeline/src/mask-recovery-mesh.ts";
import { sceneToGame, gltfToScene } from "../shared/src/geometry.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";

const [stage, mode] = process.argv.slice(2);
assert.ok(stage);
assert.ok(mode === undefined || mode === "--all-surfaces");
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
const camera = { kind: "oblique-orthographic", elevation_deg: 35 };
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
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
  const dx = b[0] - a[0],
    dy = b[1] - a[1];
  const t = Math.max(
    0,
    Math.min(1, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / (dx * dx + dy * dy || 1)),
  );
  return Math.hypot(p[0] - a[0] - t * dx, p[1] - a[1] - t * dy);
}
const report = [];
for (const edit of edits) {
  const surfaces = edit.gameplay.surfaces.filter(
    (surface) => surface.projectionVolume || mode === "--all-surfaces",
  );
  if (!surfaces.length) continue;
  const entry = index.find((entry) => entry.id === edit.asset);
  const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
  assert.equal(hash(bytes), edit.descriptorSha256);
  const descriptor = JSON.parse(bytes);
  const modelSha256 = hash(await fs.readFile(`library/3d-assets/${entry.model}`));
  const model = await loadSceneModel("library", {
    id: edit.asset,
    role: "objects",
    model: `3d-assets/${entry.model}`,
    model_sha256: modelSha256,
    descriptor: `3d-assets/${entry.descriptor}`,
    descriptor_sha256: edit.descriptorSha256,
    resources: descriptor.resources ?? [],
    ...(entry.model_scene ? { model_scene: entry.model_scene } : {}),
  });
  for (const surface of surfaces) {
    const plane = heightPlane(
      surface.polygon.map(([x, y], i) => [
        x,
        y,
        Array.isArray(surface.height) ? surface.height[i] : surface.height,
      ]),
    );
    const triangles = maskRecoveryMesh(model, surface.node, (p) =>
      sceneToGame(camera, gltfToScene(p)),
    );
    // Limit gap measurements to faces near the receiving plane. Distant walls
    // and the underside must not disguise a missing visible walking surface.
    const top = triangles.filter((triangle) =>
      triangle.every((p) => Math.abs(p[2] - planeHeight(plane, p)) < 0.5),
    );
    assert.ok(top.length, `No near-floor mesh: ${edit.asset}/${surface.id}`);
    const bounds = [0, 1].map((axis) => [
      Math.min(...surface.polygon.map((p) => p[axis])),
      Math.max(...surface.polygon.map((p) => p[axis])),
    ]);
    const points = surface.polygon.flatMap((a, i) =>
      Array.from({ length: 41 }, (_, j) =>
        a.map(
          (v, axis) => v + ((surface.polygon[(i + 1) % surface.polygon.length][axis] - v) * j) / 40,
        ),
      ),
    );
    for (let x = 0; x < 40; x++)
      for (let y = 0; y < 40; y++) {
        const point = [x, y].map(
          (v, axis) => bounds[axis][0] + ((v + 0.5) / 40) * (bounds[axis][1] - bounds[axis][0]),
        );
        if (
          pointInGameplayPolygon(point, surface.polygon, true) &&
          !(surface.holes ?? []).some((hole) => pointInGameplayPolygon(point, hole, true))
        )
          points.push(point);
      }
    const samples = points.map((point) => {
      const heights = top.map((triangle) => height(point, triangle)).filter((v) => v !== undefined);
      return {
        point,
        supported: heights.length > 0,
        heightError: heights.length
          ? Math.min(...heights.map((z) => Math.abs(z - planeHeight(plane, point))))
          : null,
        gap: heights.length
          ? 0
          : Math.min(
              ...top.flatMap((triangle) =>
                triangle.map((a, i) => distance(point, a, triangle[(i + 1) % 3])),
              ),
            ),
      };
    });
    report.push({
      asset: edit.asset,
      surface: surface.id,
      modelSha256,
      samples,
      supported: samples.filter((sample) => sample.supported).length,
      maximumGap: Math.max(...samples.map((sample) => sample.gap)),
      maximumHeightError: Math.max(...samples.map((sample) => sample.heightError ?? 0)),
    });
  }
}
await fs.writeFile(
  `${stage}/receiving-floor-mesh-review.json`,
  JSON.stringify({
    scope: "sampled near-plane mesh coverage, not rendered actor verification",
    report,
  }),
);
console.log(
  JSON.stringify(report.map(({ samples, ...result }) => ({ ...result, samples: samples.length }))),
);
