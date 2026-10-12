// Stage a shared asset-local boundary; publication requires placement and mesh review.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { gameToScene } from "../shared/src/scene.ts";
import { applyAffineMatrix, sceneToGame } from "../shared/src/geometry.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";

const scene = await readStoredMap("library/scenes/nottingham.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", scene.assetSources, scene.sceneAssets);
const definitions = [
  ["nottingham-north-street-roof-edge-slab", "building-140-walk-0", [1, 2]],
  ["nottingham-north-stone-house", "building-113-walk-0", [9, 8]],
];
const selected = definitions.map(([id, surfaceId, indices]) => {
  const asset = assets.get(id);
  const surface = asset.gameplay.surfaces.find((entry) => entry.id === surfaceId);
  const part = scene.objects.find((entry) => entry.node === `asset:${id}:${surface.node}`);
  const matrix = partMatrix(scene.camera, scene, part);
  const world = (p) =>
    sceneToGame(scene.camera, applyAffineMatrix(matrix, gameToScene(scene.camera, ...p)));
  const origin = world([0, 0, 0]);
  // This authoring extraction requires untranslated axes. The resulting sockets
  // are local definitions and are subsequently tested under independent placement.
  for (const axis of [0, 1, 2]) {
    const p = [0, 0, 0];
    p[axis] = 1;
    assert.ok(world(p).every((n, i) => Math.abs(n - origin[i] - p[i]) < 1e-7));
  }
  return { id, asset, surface, indices, origin, before: structuredClone(surface) };
});
const [slab, roof] = selected;
const changes = [];
for (const [i, index] of slab.indices.entries()) {
  const other = roof.indices[i];
  const point = [...roof.surface.polygon[other], roof.surface.height[other]].map(
    (n, axis) => n + roof.origin[axis] - slab.origin[axis],
  );
  const before = [...slab.surface.polygon[index], slab.surface.height[index]];
  const distance = Math.hypot(...point.map((n, axis) => n - before[axis]));
  assert.ok(distance < 2, `Contact adjustment needs separate review: ${distance}`);
  slab.surface.polygon[index] = point.slice(0, 2);
  slab.surface.height[index] = point[2];
  changes.push({ index, before, after: point, distance });
}
// Keep a planar receiving surface after aligning its contact edge. Record the
// opposite-corner adjustment for the same pending mesh review as the seam.
const slabPlane = heightPlane(
  slab.surface.polygon.slice(0, 3).map(([x, y], i) => [x, y, slab.surface.height[i]]),
);
const previousHeight = slab.surface.height[3];
slab.surface.height[3] = planeHeight(slabPlane, slab.surface.polygon[3]);
assert.ok(Math.abs(slab.surface.height[3] - previousHeight) < 2);
changes.push({ index: 3, beforeHeight: previousHeight, afterHeight: slab.surface.height[3] });
const slabPoints = slab.surface.polygon.map(([x, y], i) => [x, y, slab.surface.height[i]]);
slab.surface.projectionMaterials.planePoints = slabPoints.slice(0, 3);
slab.surface.projectionMaterials.footprint = slabPoints;
slab.surface.projectionMaterials.priorityHeight = Math.max(...slab.surface.height);
const projectedPlane = heightPlane(slabPoints.map(([x, y, z]) => [x, y - z, z]));
const zone = slab.asset.gameplay.jumpZones.find((entry) => entry.id === "jump-zone-34");
const beforeAnchor = [...zone.anchor];
const projectedY = zone.anchor[1] - zone.anchor[2];
const anchorHeight = planeHeight(projectedPlane, [zone.anchor[0], projectedY]);
zone.anchor = [zone.anchor[0], projectedY + anchorHeight, anchorHeight];
changes.push({ beforeAnchor, afterAnchor: zone.anchor });
// The previous floor-clearance slices describe the old slab plane. Replace
// them together with the candidate floor; collision ownership still needs review.
const beforeClearances = structuredClone(slab.asset.gameplay.movementClearances);
slab.asset.gameplay.movementClearances = [
  {
    id: "building-140-contact-floor-clearance",
    node: slab.surface.node,
    polygon: structuredClone(slab.surface.polygon),
    height: [...slab.surface.height],
    preserveMovementPrecision: true,
  },
];
changes.push({ beforeClearances, afterClearances: slab.asset.gameplay.movementClearances });
// Stage a matching movement-only support volume. The original part remains the
// visual/query geometry; compare both against the mesh before publication.
const originalShape = slab.asset.parts.find(
  (part) => part.node === slab.surface.node,
).obstacle_local_game;
assert.ok(
  originalShape.points.every((point) => point.z_bottom === originalShape.points[0].z_bottom),
);
assert.ok(!slab.asset.gameplay.volumes?.length && !slab.asset.gameplay.movementSolids);
const { projection_area: _projectionArea, material_indices: _materials, ...shape } = originalShape;
const support = {
  id: "building-140-contact-support",
  node: slab.surface.node,
  shape: {
    ...shape,
    mouse: false,
    points: slabPoints.map(([x, y, z]) => ({
      x,
      y,
      z_bottom: originalShape.points[0].z_bottom,
      z_top: z,
    })),
  },
};
slab.asset.gameplay.volumes = [support];
slab.asset.gameplay.movementSolids = [support.id];
changes.push({ beforeSupport: originalShape, afterSupport: support });
const edits = [];
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
for (const { id, asset, surface, indices } of selected) {
  assert.ok(!surface.navigationJoins);
  surface.navigationRegion = surface.id;
  surface.navigationJoins = [
    indices.map((index) => [...surface.polygon[index], surface.height[index]]),
  ];
  surface.preserveMovementPrecision = true;
  surface.preserveMovementBoundary = true;
  validateAssetGameplay(asset.gameplay, asset);
  const root = `library/3d-assets/nottingham/${id}`;
  edits.push({
    asset: id,
    descriptorSha256: hash(await fs.readFile(`${root}/asset.json`)),
    modelSha256: hash(await fs.readFile(`${root}/${asset.model}`)),
    gameplay: asset.gameplay,
  });
}
const output = await fs.mkdtemp("work/needs-review/map-compile/roof-slab-contact-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({
    status: "unpublished: requires mesh and moved-placement validation",
    changes,
    surfaces: selected.map(({ id, before, surface }) => ({ id, before, after: surface })),
  }),
);
console.log(output);
