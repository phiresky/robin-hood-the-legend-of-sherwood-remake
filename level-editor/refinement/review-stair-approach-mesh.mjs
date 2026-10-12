// Inspect prepared outdoor stair approaches against the placed visible meshes.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh } from "../pipeline/src/mask-recovery-mesh.ts";
import { closedMeshComponents } from "../pipeline/src/closed-mesh-components.ts";
import { meshCappedVolumes } from "../pipeline/src/mesh-capped-volumes.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { gltfToScene, sceneToGame, applyAffineMatrix } from "../shared/src/geometry.ts";
import { gameToScene } from "../shared/src/scene.ts";
import { NAVIGATION_HALF_DIAGONAL } from "../shared/src/navigation-footprint.ts";
import { footprintBoundaryHull } from "../shared/src/supported-footprint-centers.ts";

// Clip a triangle against the entire swept footprint prism, rather than only
// casting corner rays that can miss a narrow face enclosed by the footprint.
function prismIntersection(triangle, boundary, bottom, top) {
  const planes = [
    [0, 0, 1, -bottom],
    [0, 0, -1, top],
  ];
  for (const [i, a] of boundary.entries()) {
    const b = boundary[(i + 1) % boundary.length];
    const dx = b[0] - a[0],
      dy = b[1] - a[1];
    planes.push([-dy, dx, 0, dy * a[0] - dx * a[1]]);
  }
  let polygon = triangle;
  for (const [a, b, c, d] of planes) {
    const output = [];
    for (const [i, p] of polygon.entries()) {
      const q = polygon[(i + 1) % polygon.length];
      const dp = a * p[0] + b * p[1] + c * p[2] + d;
      const dq = a * q[0] + b * q[1] + c * q[2] + d;
      if (dp >= 0) output.push(p);
      if (dp >= 0 !== dq >= 0) {
        const t = dp / (dp - dq);
        output.push(p.map((v, axis) => v + t * (q[axis] - v)));
      }
    }
    polygon = output;
  }
  return (
    polygon.length >= 3 &&
    polygon
      .slice(1, -1)
      .some(
        (p, i) =>
          Math.hypot(...cross(subtract(p, polygon[0]), subtract(polygon[i + 2], polygon[0]))) >
          1e-8,
      )
  );
}

const subtract = (a, b) => a.map((v, i) => v - b[i]);
const dot = (a, b) => a.reduce((sum, v, i) => sum + v * b[i], 0);
const cross = (a, b) => [
  a[1] * b[2] - a[2] * b[1],
  a[2] * b[0] - a[0] * b[2],
  a[0] * b[1] - a[1] * b[0],
];
function intersection(start, end, [a, b, c]) {
  const direction = subtract(end, start);
  const ab = subtract(b, a),
    ac = subtract(c, a);
  const p = cross(direction, ac);
  const determinant = dot(ab, p);
  if (Math.abs(determinant) < 1e-9) return undefined;
  const offset = subtract(start, a);
  const u = dot(offset, p) / determinant;
  if (u < -1e-7 || u > 1 + 1e-7) return undefined;
  const q = cross(offset, ab);
  const v = dot(direction, q) / determinant;
  if (v < -1e-7 || u + v > 1 + 1e-7) return undefined;
  const t = dot(ac, q) / determinant;
  return t > 1e-7 && t < 1 - 1e-7 ? t : undefined;
}
// Check a crossing, reverse winding, a miss and a parallel approach.
const triangle = [
  [0, -2, -2],
  [0, 2, -2],
  [0, 0, 2],
];
assert.equal(intersection([-1, 0, 0], [1, 0, 0], triangle), 0.5);
assert.equal(intersection([-1, 0, 0], [1, 0, 0], triangle.toReversed()), 0.5);
assert.equal(intersection([-1, 5, 0], [1, 5, 0], triangle), undefined);
assert.equal(intersection([1, -1, 0], [1, 1, 0], triangle), undefined);
const square = [
  [-1, -1],
  [1, -1],
  [1, 1],
  [-1, 1],
];
assert.ok(
  prismIntersection(
    [
      [0, 0, 1],
      [0.1, 0, 1],
      [0, 0.1, 1],
    ],
    square,
    0,
    2,
  ),
);
assert.ok(
  !prismIntersection(
    [
      [0, 0, 3],
      [0.1, 0, 3],
      [0, 0.1, 3],
    ],
    square,
    0,
    2,
  ),
);
assert.ok(
  !prismIntersection(
    [
      [2, 0, 1],
      [3, 0, 1],
      [2, 1, 1],
    ],
    square,
    0,
    2,
  ),
);

const [directory, stage, modelStage, stepModelStage] = process.argv.slice(2);
assert.ok(directory && stage, "Supply isolated stair exports and their authoring stage");
const manifest = JSON.parse(await fs.readFile(`${directory}/diagnostics.json`));
assert.equal(manifest.complete, true);
assert.equal(manifest.gameplayStage, stage, "Review the same asset stage used by the exporter");
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`));
const stairEdits = edits.filter((entry) => entry.asset === "lincoln-south-wall-walk-step");
assert.equal(stairEdits.length, 1);
const edit = stairEdits[0];
assert.equal(edit.gameplay.lifts.length, 1);
const modelCandidate = modelStage
  ? {
      config: JSON.parse(await fs.readFile(`${modelStage}/opening.json`)),
      report: JSON.parse(await fs.readFile(`${modelStage}/mesh-build.json`)),
    }
  : undefined;
const stepCandidate = stepModelStage
  ? {
      config: JSON.parse(await fs.readFile(`${stepModelStage}/mesh-plane.json`)),
      report: JSON.parse(await fs.readFile(`${stepModelStage}/mesh-build.json`)),
    }
  : undefined;
const models = new Map();
const serializedSolids = [];
const results = [];
for (const { file } of manifest.results) {
  const document = JSON.parse(await fs.readFile(`${directory}/${file}.scene.json`));
  const source = document.assetSources.find((source) => source.id === edit.asset);
  assert.equal(source?.descriptor_sha256, edit.descriptorSha256);
  assert.equal(source?.model_sha256, edit.modelSha256);
  const geometry = JSON.parse(await fs.readFile(`${directory}/${file}`)).asset_geometry;
  assert.equal(geometry.lifts.length, 1);
  const meshes = [];
  for (const source of document.assetSources) {
    if (!models.has(source.id)) {
      if (modelCandidate?.config.asset === source.id) {
        assert.equal(modelCandidate.config.source.model_sha256, source.model_sha256);
        assert.equal(modelCandidate.config.source.descriptor_sha256, source.descriptor_sha256);
        models.set(
          source.id,
          await loadSceneModel(modelStage, {
            id: source.id,
            role: "objects",
            model: "model.glb",
            model_scene: source.model_scene,
            model_sha256: modelCandidate.report.modelSha256,
            resources: [],
          }),
        );
        for (const part of JSON.parse(await fs.readFile(`library/${source.descriptor}`)).parts) {
          const mesh = maskRecoveryMesh(models.get(source.id), part.node, (point) =>
            sceneToGame(document.camera, gltfToScene(point)),
          );
          const components = closedMeshComponents(mesh);
          const volumes = components.flatMap((component) => meshCappedVolumes(component));
          serializedSolids.push({
            node: part.node,
            components: components.length,
            volumes: volumes.length,
          });
        }
      } else if (stepCandidate && source.id === edit.asset) {
        assert.equal(stepCandidate.config.modelSha256, source.model_sha256);
        models.set(
          source.id,
          await loadSceneModel(stepModelStage, {
            id: source.id,
            role: "objects",
            model: "model.glb",
            model_scene: source.model_scene,
            model_sha256: stepCandidate.report.modelSha256,
            resources: [],
          }),
        );
      } else models.set(source.id, await loadSceneModel("library", source));
    }
    for (const object of document.objects.filter((object) =>
      object.node.startsWith(`asset:${source.id}:`),
    )) {
      const matrix = partMatrix(document.camera, document, object);
      meshes.push({
        object: object.id,
        asset: source.id,
        triangles: maskRecoveryMesh(
          models.get(source.id),
          object.node.slice(`asset:${source.id}:`.length),
          (point) => sceneToGame(document.camera, applyAffineMatrix(matrix, gltfToScene(point))),
        ),
      });
    }
  }
  const authored = edit.gameplay.lifts[0];
  const object = document.objects.find(
    (object) => object.node === `asset:${edit.asset}:${authored.node}`,
  );
  assert.ok(object);
  const matrix = partMatrix(document.camera, document, object);
  const doors = geometry.lifts[0].doors;
  assert.equal(doors.length, authored.doors.length);
  for (const [index, door] of doors.entries()) {
    const outside = sceneToGame(
      document.camera,
      applyAffineMatrix(matrix, gameToScene(document.camera, ...authored.doors[index].outside)),
    );
    const middle = sceneToGame(
      document.camera,
      applyAffineMatrix(matrix, gameToScene(document.camera, ...authored.doors[index].middle)),
    );
    assert.ok(
      Math.abs(outside[2] - middle[2]) < 1e-5,
      "This diagnostic requires a level outside approach",
    );
    // Use the final prepared screen endpoints on the authored landing plane.
    // One unit above the floor avoids counting contact with the walking mesh.
    const z = outside[2];
    const start = [door.point_out[0], door.point_out[1] + z, z + 1];
    const end = [door.point_mid[0], door.point_mid[1] + z, z + 1];
    const hits = [];
    const doorwayHits = [];
    const [halfX, halfY] = NAVIGATION_HALF_DIAGONAL;
    const boundary = footprintBoundaryHull(
      [start, end].flatMap(([x, y]) => [
        [x - halfX, y - halfY],
        [x + halfX, y - halfY],
        [x + halfX, y + halfY],
        [x - halfX, y + halfY],
      ]),
    );
    for (const mesh of meshes) {
      if (mesh.asset === modelCandidate?.config.asset) {
        const point = [start[0], start[1], z + 20];
        const bottom =
          Math.min(...mesh.triangles.flatMap((triangle) => triangle.map((p) => p[2]))) - 1;
        const ray = [point[0], point[1], bottom];
        const crossings = mesh.triangles
          .map((triangle) => intersection(ray, point, triangle))
          .filter((t) => t !== undefined)
          .sort((a, b) => a - b)
          .filter((t, i, values) => i === 0 || Math.abs(t - values[i - 1]) > 1e-7);
        if (crossings.length % 2)
          doorwayHits.push({ asset: mesh.asset, object: mesh.object, enclosedStart: point });
      }
      for (const [triangle, points] of mesh.triangles.entries()) {
        const t = intersection(start, end, points);
        if (t !== undefined)
          hits.push({
            asset: mesh.asset,
            object: mesh.object,
            triangle,
            t,
            point: start.map((v, axis) => v + (end[axis] - v) * t),
          });
        if (
          mesh.asset === modelCandidate?.config.asset &&
          prismIntersection(points, boundary, z + 0.1, z + 40)
        )
          doorwayHits.push({ asset: mesh.asset, object: mesh.object, triangle });
      }
    }
    results.push({ file, door: index, start, end, hits, doorwayHits });
  }
}
const report = {
  scope:
    "prepared-outdoor-stair-centerline-one-unit-above-floor-vs-mesh-not-full-actor-or-animation",
  audit_finished: true,
  ...(modelStage ? { unpublishedModelStage: modelStage, serializedSolids } : {}),
  ...(stepModelStage ? { unpublishedStepModelStage: stepModelStage } : {}),
  clear: results.every((result) => result.hits.length === 0),
  approaches: results.length,
  crossingApproaches: results.filter((result) => result.hits.length > 0).length,
  ...(modelCandidate
    ? {
        doorwayEnvelope: {
          half: NAVIGATION_HALF_DIAGONAL,
          height: 40,
          scope:
            "turret-only-swept-native-footprint-and-standing-visibility-height-not-sprite-animation",
          clear: results.every((result) => result.doorwayHits.length === 0),
          crossingApproaches: results.filter((result) => result.doorwayHits.length > 0).length,
        },
      }
    : {}),
  results,
};
await fs.writeFile(
  `${directory}/stair-approach-mesh${modelStage ? "-candidate" : ""}-report.json`,
  JSON.stringify(report),
);
console.log(JSON.stringify({ ...report, results: undefined }));
if (!report.clear || report.doorwayEnvelope?.clear === false) process.exitCode = 1;
