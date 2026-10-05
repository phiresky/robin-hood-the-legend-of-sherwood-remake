import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { createRequire } from "node:module";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh, maskRecoveryTextures } from "../pipeline/src/mask-recovery-mesh.ts";
import { gltfToScene, sceneToGame } from "../shared/src/geometry.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

// Explicit authoring candidate, never an automatic export repair. The revised
// flight and its landing still require mesh coverage and native route review.
const asset = "derby-upper-west-curtain";
const require = createRequire(new URL("../shared/package.json", import.meta.url));
const polygonClipping = require("polygon-clipping");
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
const entry = index.find((entry) => entry.id === asset);
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
assert.equal(hash(bytes), entry.descriptor_sha256);
const descriptor = JSON.parse(bytes);
const gameplay = structuredClone(descriptor.gameplay);
const lift = gameplay.lifts.find((lift) => lift.id === "building-114-lift");
const floor = gameplay.surfaces.find((surface) => surface.id === lift.surface);
const landing = gameplay.surfaces.find((surface) => surface.id === "building-128-walk-0");
const low = lift.doors[0].outside[2],
  high = lift.doors[1].outside[2];
assert.ok(high > low && landing.height.every((z) => z === high));
const modelBytes = await fs.readFile(`library/3d-assets/${entry.model}`);
const model = await loadSceneModel("library", {
  id: asset,
  role: "objects",
  descriptor: `3d-assets/${entry.descriptor}`,
  descriptor_sha256: entry.descriptor_sha256,
  model: `3d-assets/${entry.model}`,
  model_sha256: hash(modelBytes),
  resources: descriptor.resources ?? [],
  ...(entry.model_scene ? { model_scene: entry.model_scene } : {}),
});
const triangles = maskRecoveryMesh(
  model,
  lift.node,
  (p) => sceneToGame({ kind: "oblique-orthographic", elevation_deg: 35 }, gltfToScene(p)),
  await maskRecoveryTextures(model),
);
// The underside identifies the complete flight footprint without treating a
// riser's upper edge as the ground approach or stopping before the last tread.
const bottom = triangles.filter((triangle) => triangle.every((p) => Math.abs(p[2] - low) < 0.02));
assert.equal(bottom.length, 2, "Reviewed flight underside changed");
const points = [];
for (const point of bottom.flat())
  if (!points.some((p) => Math.hypot(p[0] - point[0], p[1] - point[1]) < 1e-4)) points.push(point);
assert.equal(points.length, 4);
const oldPlane = heightPlane(floor.polygon.map(([x, y], i) => [x, y, floor.height[i]]));
points.sort((a, b) => planeHeight(oldPlane, a) - planeHeight(oldPlane, b));
const lower = points.slice(0, 2),
  upper = points.slice(2);
const top = [0, 1].map((axis) => (upper[0][axis] + upper[1][axis]) / 2);
const plane = heightPlane([
  [lower[0][0], lower[0][1], low],
  [lower[1][0], lower[1][1], low],
  [...top, high],
]);
const seat = (point, z) => {
  const t = (z - planeHeight(plane, point)) / (plane[0] ** 2 + plane[1] ** 2);
  return [point[0] + t * plane[0], point[1] + t * plane[1]];
};
// Retain the tread outline rather than the wider underside. Quantization here
// only joins duplicate mesh vertices whose decoded positions differ by microns.
const treadTriangles = triangles.filter(
  (triangle) =>
    triangle.every((p) => p[2] > low + 0.1) &&
    Math.max(...triangle.map((p) => p[2])) - Math.min(...triangle.map((p) => p[2])) < 0.01,
);
const coverage = polygonClipping.union(
  ...treadTriangles.map((triangle) => [
    triangle.map((p) => p.slice(0, 2).map((v) => Math.round(v * 1000) / 1000)),
  ]),
);
assert.equal(coverage.length, 1, "Tread coverage is disconnected");
assert.equal(coverage[0].length, 1, "Tread coverage has holes");
const polygon = coverage[0][0].slice(0, -1).map((point) => {
  const z = planeHeight(plane, point);
  if (Math.abs(z - low) < 0.25) return seat(point, low);
  if (upper.some((p) => Math.hypot(p[0] - point[0], p[1] - point[1]) < 0.002))
    return seat(point, high);
  return point;
});
floor.polygon = polygon;
floor.height = floor.polygon.map((p) => planeHeight(plane, p));
floor.preserveMovementPrecision = true;
floor.projectionMaterials.planePoints = [
  [lower[0][0], lower[0][1], low],
  [lower[1][0], lower[1][1], low],
  [...top, high],
];
floor.projectionMaterials.footprint = floor.polygon.map((p, i) => [...p, floor.height[i]]);
// Move the platform's two receiving corners onto the new upper seam. The
// remaining perimeter, including its independent notch, retains its geometry.
const landingBefore = structuredClone(landing.polygon);
for (const i of [3, 4]) landing.polygon[i] = seat(landing.polygon[i], high);
landing.preserveMovementPrecision = true;
for (const [i, door] of lift.doors.entries()) {
  const z = i ? high : low;
  const seam = seat(door.middle, z);
  const length = Math.hypot(plane[0], plane[1]);
  const direction = [plane[0] / length, plane[1] / length];
  const sign = i ? -1 : 1;
  door.middle = [...seam, z];
  const inside = seam.map((v, axis) => v + sign * direction[axis] * 8);
  door.inside = [...inside, planeHeight(plane, inside)];
  door.outside = [...seam.map((v, axis) => v - sign * direction[axis] * 12), z];
}
gameplay.movementClearances ??= [];
gameplay.movementClearances.push({
  id: `${floor.id}-mesh-clearance`,
  node: floor.node,
  polygon: structuredClone(floor.polygon),
  height: [...floor.height],
  holes: [],
});
validateAssetGameplay(gameplay, descriptor);
const output = await fs.mkdtemp("work/map-compile/upper-west-mesh-floor-");
await fs.writeFile(
  `${output}/edits.json`,
  JSON.stringify([{ asset, descriptorSha256: entry.descriptor_sha256, gameplay }]),
);
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify(
    {
      scope: "unpublished mesh-derived stair floor",
      descriptorSha256: entry.descriptor_sha256,
      modelSha256: hash(modelBytes),
      oldPlane,
      plane,
      changes: lift.doors.map((door, i) => ({
        door: door.id,
        before: descriptor.gameplay.lifts.find((value) => value.id === lift.id).doors[i].middle,
        after: door.middle,
        landing: i ? landing.id : "external placement receiver",
      })),
      undersideCorners: points,
      landingBefore,
      landingAfter: landing.polygon,
      doors: lift.doors,
    },
    null,
    2,
  ),
);
console.log(JSON.stringify({ output, plane }));
