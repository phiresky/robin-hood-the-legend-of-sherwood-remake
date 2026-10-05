import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { createRequire } from "node:module";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh, maskRecoveryTextures } from "../pipeline/src/mask-recovery-mesh.ts";
import { sceneToGame, gltfToScene } from "../shared/src/geometry.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";

const [directory] = process.argv.slice(2);
assert.ok(directory, "Provide staged component edits directory");
const require = createRequire(new URL("../pipeline/package.json", import.meta.url));
const sharp = require("sharp");
const edits = JSON.parse(await fs.readFile(`${directory}/edits.json`, "utf8"));
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
const camera = { kind: "oblique-orthographic", elevation_deg: 35 };
const report = [];
// Compare smooth navigation ramps with the actual upper mesh, including treads.
function contains(point, polygon) {
  let inside = false;
  for (let i = 0, j = polygon.length - 1; i < polygon.length; j = i++) {
    const a = polygon[i],
      b = polygon[j];
    if (
      a[1] > point[1] !== b[1] > point[1] &&
      point[0] < ((b[0] - a[0]) * (point[1] - a[1])) / (b[1] - a[1]) + a[0]
    )
      inside = !inside;
  }
  return inside;
}
function meshHeight(point, triangle) {
  const [a, b, c] = triangle;
  const denominator = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
  if (Math.abs(denominator) < 1e-8) return undefined;
  const u = ((b[1] - c[1]) * (point[0] - c[0]) + (c[0] - b[0]) * (point[1] - c[1])) / denominator;
  const v = ((c[1] - a[1]) * (point[0] - c[0]) + (a[0] - c[0]) * (point[1] - c[1])) / denominator;
  if (u < -1e-8 || v < -1e-8 || u + v > 1 + 1e-8) return undefined;
  return u * a[2] + v * b[2] + (1 - u - v) * c[2];
}
function edgeDistance(point, a, b) {
  const dx = b[0] - a[0],
    dy = b[1] - a[1];
  const t = Math.max(
    0,
    Math.min(1, ((point[0] - a[0]) * dx + (point[1] - a[1]) * dy) / (dx * dx + dy * dy || 1)),
  );
  return Math.hypot(point[0] - a[0] - t * dx, point[1] - a[1] - t * dy);
}
for (const edit of edits) {
  const entry = index.find((entry) => entry.id === edit.asset);
  const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
  assert.equal(createHash("sha256").update(bytes).digest("hex"), edit.descriptorSha256);
  const descriptor = JSON.parse(bytes);
  const modelBytes = await fs.readFile(`library/3d-assets/${entry.model}`);
  const modelSha256 = createHash("sha256").update(modelBytes).digest("hex");
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
  const textures = await maskRecoveryTextures(model);
  for (const lift of edit.gameplay.lifts) {
    const before = descriptor.gameplay.surfaces.find((surface) => surface.id === lift.surface);
    const after = edit.gameplay.surfaces.find((surface) => surface.id === lift.surface);
    const plane = heightPlane(
      after.polygon.map(([x, y], i) => [
        x,
        y,
        typeof after.height === "number" ? after.height : after.height[i],
      ]),
    );
    const triangles = maskRecoveryMesh(
      model,
      lift.node,
      (p) => sceneToGame(camera, gltfToScene(p)),
      textures,
    );
    const floorTriangles = triangles.filter((triangle) =>
      triangle.every(([x, y, z]) => Math.abs(z - planeHeight(plane, [x, y])) < 2),
    );
    const samples = [];
    const bounds = [0, 1].map((axis) => [
      Math.min(...after.polygon.map((p) => p[axis])),
      Math.max(...after.polygon.map((p) => p[axis])),
    ]);
    for (let ix = 0; ix < 40; ix++)
      for (let iy = 0; iy < 40; iy++) {
        const point = [
          bounds[0][0] + ((ix + 0.5) / 40) * (bounds[0][1] - bounds[0][0]),
          bounds[1][0] + ((iy + 0.5) / 40) * (bounds[1][1] - bounds[1][0]),
        ];
        if (!contains(point, after.polygon)) continue;
        const hits = triangles
          .map((triangle) => meshHeight(point, triangle))
          .filter((z) => z !== undefined);
        samples.push({
          point,
          floor: planeHeight(plane, point),
          mesh: hits.length ? Math.max(...hits) : null,
        });
      }
    const residuals = samples
      .filter((sample) => sample.mesh !== null)
      .map((sample) => sample.mesh - sample.floor)
      .sort((a, b) => a - b);
    const uncoveredDistances = samples
      .filter((sample) => sample.mesh === null)
      .map((sample) =>
        Math.min(
          ...triangles.flatMap((triangle) =>
            triangle.map((a, i) => edgeDistance(sample.point, a, triangle[(i + 1) % 3])),
          ),
        ),
      );
    const treadHeights = [
      ...new Set(
        triangles
          .filter(
            (triangle) =>
              Math.max(...triangle.map((p) => p[2])) - Math.min(...triangle.map((p) => p[2])) <
              0.01,
          )
          .map((triangle) => Math.round(triangle[0][2] * 100) / 100),
      ),
    ].sort((a, b) => a - b);
    const points = [
      ...before.polygon,
      ...after.polygon,
      ...floorTriangles.flat().map(([x, y]) => [x, y]),
    ];
    const x0 = Math.min(...points.map((p) => p[0])) - 4,
      x1 = Math.max(...points.map((p) => p[0])) + 4;
    const y0 = Math.min(...points.map((p) => p[1])) - 4,
      y1 = Math.max(...points.map((p) => p[1])) + 4;
    const scale = Math.min(700 / (x1 - x0), 440 / (y1 - y0));
    const xy = (points) =>
      points.map(([x, y]) => `${40 + (x - x0) * scale},${65 + (y - y0) * scale}`).join(" ");
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="800" height="570">
      <rect width="800" height="570" fill="#171d25"/>
      <text x="25" y="28" fill="white" font-family="sans-serif" font-size="18">${edit.asset} / ${lift.node}</text>
      ${triangles.map((triangle) => `<polygon points="${xy(triangle)}" fill="#566171" fill-opacity="0.08" stroke="#718096" stroke-opacity="0.3" stroke-width="0.5"/>`).join("")}
      ${floorTriangles.map((triangle) => `<polygon points="${xy(triangle)}" fill="#566171" fill-opacity="0.3" stroke="#718096" stroke-width="0.7"/>`).join("")}
      <polygon points="${xy(before.polygon)}" fill="none" stroke="#ffb347" stroke-width="2"/>
      <polygon points="${xy(after.polygon)}" fill="none" stroke="#37d7df" stroke-width="2"/>
      ${lift.doors.map((door) => `<circle cx="${40 + (door.middle[0] - x0) * scale}" cy="${65 + (door.middle[1] - y0) * scale}" r="4" fill="#f36a94"/>`).join("")}
      <text x="25" y="535" fill="white" font-family="sans-serif" font-size="15">Gray: full mesh (bright: near floor). Orange: published floor. Cyan: candidate.</text>
      <text x="25" y="557" fill="white" font-family="sans-serif" font-size="15">Pink: corrected door midpoints. Top view in asset-local game coordinates.</text></svg>`;
    const file = `${directory}/${lift.node}-seam-review`;
    await fs.writeFile(`${file}.svg`, svg);
    await sharp(Buffer.from(svg)).png().toFile(`${file}.png`);
    report.push({
      asset: edit.asset,
      node: lift.node,
      modelSha256,
      meshTriangles: triangles.length,
      sampledFloorPoints: samples.length,
      sampledMeshHits: residuals.length,
      maximumUncoveredMeshEdgeDistance: Math.max(0, ...uncoveredDistances),
      treadHeights,
      meshMinusFloorQuantiles: [0, 0.1, 0.5, 0.9, 1].map(
        (q) => residuals[Math.round(q * (residuals.length - 1))],
      ),
      sampleFile: `${file}-samples.json`,
      nearFloorTriangles: floorTriangles.length,
      maximumFloorXYShift: Math.max(
        ...before.polygon.map((p, i) =>
          Math.hypot(p[0] - after.polygon[i][0], p[1] - after.polygon[i][1]),
        ),
      ),
      plane,
      images: [`${file}.svg`, `${file}.png`],
    });
    await fs.writeFile(`${file}-samples.json`, JSON.stringify(samples, null, 2));
  }
}
await fs.writeFile(`${directory}/mesh-review.json`, JSON.stringify(report, null, 2));
console.log(JSON.stringify(report, null, 2));
