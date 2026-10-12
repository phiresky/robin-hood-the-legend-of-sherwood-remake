// Cut an unpublished doorway into authored asset-local wall solids.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { fixedPolygonBoolean } from "../shared/src/fixed-polygon-boolean.ts";
import { prepareSightCap } from "../shared/src/prepare-sight-cap.ts";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh } from "../pipeline/src/mask-recovery-mesh.ts";
import { gltfToScene, sceneToGame } from "../shared/src/geometry.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const [modelStage, stepStage] = process.argv.slice(2);
assert.ok(modelStage && stepStage);
const config = JSON.parse(await fs.readFile(`${modelStage}/opening.json`));
const build = JSON.parse(await fs.readFile(`${modelStage}/mesh-build.json`));
assert.ok(build.changed.every((part) => part.nonManifoldEdges === 0));
const bytes = await fs.readFile(`library/${config.source.descriptor}`);
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
assert.equal(hash(bytes), config.source.descriptor_sha256);
const descriptor = JSON.parse(bytes);
assert.equal(hash(await fs.readFile(`${modelStage}/model.glb`)), build.modelSha256);
const sourceModel = await loadSceneModel("library", config.source);
const gameplay = structuredClone(descriptor.gameplay);
assert.equal(gameplay.surfaces.length, 0);
assert.equal(gameplay.doors.length, 0);
assert.equal(gameplay.lifts.length, 0);
assert.equal(gameplay.volumes?.length ?? 0, 0);
gameplay.collision = "none";
gameplay.volumes = [];
gameplay.movementClearances = gameplay.movementClearances.filter(
  (entry) => !config.nodes.includes(entry.node),
);
gameplay.sightOrder = {};
const parts = [];
const rings = (polygons) =>
  polygons.map((polygon) => {
    assert.equal(polygon.length, 1, "The through-opening must not leave enclosed holes");
    return polygon[0].slice(0, -1);
  });
for (const part of descriptor.parts) {
  const shape = part.obstacle_local_game;
  let volumes = [shape.points];
  if (config.nodes.includes(part.node)) {
    const { z_bottom, z_top } = shape.points[0];
    assert.ok(shape.points.every((p) => p.z_bottom === z_bottom && p.z_top === z_top));
    assert.ok(config.floorHeight - 0.1 < z_bottom && config.doorwayTop < z_top);
    const footprint = shape.points.map((p) => [p.x, p.y]);
    const mesh = maskRecoveryMesh(sourceModel, part.node, (point) =>
      sceneToGame(config.camera, gltfToScene(point)),
    );
    const bottomTriangles = mesh.filter((triangle) =>
      triangle.every((point) => Math.abs(point[2] - z_bottom) < 1e-3),
    );
    assert.ok(bottomTriangles.length, "The wall needs a flat authored bottom cap");
    const lowerFootprint = fixedPolygonBoolean(
      "union",
      bottomTriangles.map((triangle) => [triangle.map(([x, y]) => [x, y])]),
    );
    const outside = rings(fixedPolygonBoolean("difference", lowerFootprint, [[config.polygon]]));
    const upperOutside = rings(fixedPolygonBoolean("difference", [footprint], [[config.polygon]]));
    assert.ok(outside.length && upperOutside.length);
    const joint = 356;
    assert.ok(
      mesh.some((triangle) => triangle.every((point) => Math.abs(point[2] - joint) < 1e-3)),
      "The source wall must retain its authored stacked-shell interface",
    );
    const slab = (ring, bottom, top) =>
      ring.map(([x, y]) => ({
        x,
        y,
        z_bottom: bottom,
        z_top: top,
      }));
    volumes = [
      ...outside.map((ring) => slab(ring, z_bottom, joint)),
      ...upperOutside.map((ring) => slab(ring, joint, config.doorwayTop)),
      slab(footprint, config.doorwayTop, z_top),
    ];
  }
  if (part.node === config.floorNode)
    volumes.push(
      config.polygon.map(([x, y]) => ({
        x,
        y,
        z_bottom: config.floorHeight - 4,
        z_top: config.floorHeight,
      })),
    );
  for (const [volumeIndex, rawPoints] of volumes.entries()) {
    const id = `${part.node}/cut-${volumeIndex}`;
    const points = prepareSightCap(rawPoints, id);
    const {
      projection_area: _projection,
      material_indices: _materials,
      ...flags
    } = part.obstacle_local_game;
    gameplay.volumes.push({ id, node: part.node, shape: { ...flags, points } });
    gameplay.sightOrder[id] = descriptor.gameplay.sightOrder[part.node];
  }
  parts.push({ node: part.node, volumes: volumes.length, cut: config.nodes.includes(part.node) });
}
gameplay.surfaces = [
  {
    id: "doorway-floor",
    node: config.floorNode,
    polygon: config.polygon,
    height: config.floorHeight,
    preserveMovementPrecision: true,
    projectionMaterials: {
      defaultMaterial: 0,
      regions: [],
      planePoints: config.polygon.slice(0, 3).map(([x, y]) => [x, y, config.floorHeight]),
      footprint: config.polygon.map(([x, y]) => [x, y, config.floorHeight]),
    },
  },
];
gameplay.draft.issues.push(
  "Unpublished repaired doorway: cut wall solids and receiving floor require mesh agreement, moved-assembly/native and full actor-clearance checks; floor texturing is provisional.",
);
validateAssetGameplay(gameplay, descriptor);
const edits = JSON.parse(await fs.readFile(`${stepStage}/edits.json`));
assert.equal(edits.length, 1);
assert.equal(edits[0].asset, "lincoln-south-wall-walk-step");
edits.push({
  asset: config.asset,
  descriptorSha256: config.source.descriptor_sha256,
  modelSha256: config.source.model_sha256,
  gameplay,
});
const output = await fs.mkdtemp("work/needs-review/map-compile/turret-gameplay-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({
    scope: "unpublished-authored-wall-cut-not-native-or-visual-parity",
    modelStage,
    modelSha256: build.modelSha256,
    stepStage,
    parts,
  }),
);
console.log(output);
