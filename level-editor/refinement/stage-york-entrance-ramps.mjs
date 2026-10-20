// Author reviewed entrance ramp tops as physical floors with terrain contacts.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { sceneToGame } from "../shared/src/geometry.ts";

const document = await readStoredMap("library/scenes/york.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
const reviewed = [
  { asset: "york-central-south-red-timber-house", ramps: ["building-327"] },
  { asset: "york-west-bridge-approach-gatehouse", ramps: ["building-171", "building-177"] },
];
const selected = process.argv.slice(2);
assert.ok(
  selected.every((id) => reviewed.some((entry) => entry.asset === id)),
  "Unknown ramp asset",
);
const changed = [],
  review = [],
  fixtures = [];
for (const { asset: id, ramps } of reviewed) {
  if (selected.length && !selected.includes(id)) continue;
  const asset = assets.get(id),
    gameplay = structuredClone(asset.gameplay);
  assert.equal(gameplay.movementSolids, undefined);
  assert.ok(
    !gameplay.movementTransitions?.length,
    "State-controlled structures require separate review",
  );
  gameplay.movementSolids = asset.parts
    .filter(
      (part) =>
        part.obstacle_local_game?.solid &&
        part.collision !== "none" &&
        part.mission_profile === undefined,
    )
    .map((part) => part.node);
  assert.ok(ramps.every((node) => gameplay.movementSolids.includes(node)));
  const minimums = [],
    rampPlanes = [],
    groundApproaches = [];
  for (const node of ramps) {
    const shape = asset.parts.find((part) => part.node === node).obstacle_local_game;
    assert.ok(shape.solid && !shape.material_indices.length);
    const receiver = gameplay.projectionReceivers.find(
      (entry) => entry.node === node && entry.volume === node,
    );
    assert.ok(receiver);
    assert.ok(!gameplay.surfaces.some((surface) => surface.node === node));
    const authored = shape.points.map((point) => [point.x, point.y, point.z_top]);
    const low = Math.min(...authored.map((point) => point[2]));
    const bottom = authored.filter((point) => Math.abs(point[2] - low) < 0.001);
    assert.equal(bottom.length, 2);
    const high = authored.find((point) => point[2] > low + 1);
    assert.ok(high);
    // Keep both terrain contacts exact when normalizing rounded volume corners.
    const plane = heightPlane([...bottom, high]);
    rampPlanes.push({ node, plane });
    const vertices = authored.map(([x, y]) => [x, y, planeHeight(plane, [x, y])]);
    const correction = Math.max(
      ...vertices.map((point, index) => Math.abs(point[2] - authored[index][2])),
    );
    assert.ok(correction < 0.001);
    const minimum = Math.min(...vertices.map((point) => point[2]));
    const joins = vertices.flatMap((point, index) => {
      const next = vertices[(index + 1) % vertices.length];
      return Math.abs(point[2] - minimum) < 0.001 && Math.abs(next[2] - minimum) < 0.001
        ? [[point, next]]
        : [];
    });
    assert.equal(joins.length, 1, "Each reviewed ramp needs one level bottom edge");
    const midpoint = [0, 1].map((axis) => (joins[0][0][axis] + joins[0][1][axis]) / 2);
    const center = [0, 1].map(
      (axis) => vertices.reduce((sum, vertex) => sum + vertex[axis], 0) / vertices.length,
    );
    const direction = midpoint.map((value, axis) => value - center[axis]);
    const length = Math.hypot(...direction);
    assert.ok(length > 0);
    groundApproaches.push(midpoint.map((value, axis) => value + (direction[axis] * 24) / length));
    minimums.push(minimum);
    gameplay.surfaces.push({
      id: `${node}-entrance-ramp`,
      node,
      navigationRegion: `${node}-entrance-ramp`,
      polygon: vertices.map(([x, y]) => [x, y]),
      height: vertices.map((point) => point[2]),
      preserveMovementPrecision: true,
      navigationJoins: joins,
      projectionMaterials: { defaultMaterial: shape.default_material, regions: [] },
    });
    gameplay.projectionReceivers = gameplay.projectionReceivers.filter(
      (entry) => entry !== receiver,
    );
    review.push({ asset: id, node, receiver, correction, bottomEdge: joins[0] });
  }
  assert.ok(minimums.every((value) => Math.abs(value - minimums[0]) < 0.001));
  assert.deepEqual(
    gameplay.interiors,
    asset.gameplay.interiors,
    "Preserve all doorway anchors and rules",
  );
  assert.deepEqual(gameplay.movementBlockers, asset.gameplay.movementBlockers);
  const doors = gameplay.interiors.flatMap((room) => room.doors);
  assert.equal(doors.length, rampPlanes.length);
  const matrixFor = (node) => {
    const part = document.objects.find((object) => object.node === `asset:${id}:${node}`);
    assert.ok(part);
    const obstacle = asset.parts.find((entry) => entry.node === node).obstacle_local_game;
    return partMatrix(document.camera, document, { ...part, obstacle });
  };
  for (const [index, door] of doors.entries()) {
    const { node, plane } = rampPlanes[index];
    assert.deepEqual(
      matrixFor(door.node),
      matrixFor(node),
      "Reviewed door and ramp share a local frame",
    );
    assert.equal(door.outsideAnchor, undefined);
    const before = [...door.outside];
    // Normalize the endpoint onto its floor while retaining its projected location.
    // Exact local incidence survives rotation without a relaxed binding tolerance.
    const dz = (planeHeight(plane, before) - before[2]) / (1 - plane[1]);
    assert.ok(Number.isFinite(dz) && Math.abs(dz) < 0.001);
    door.outside[1] += dz;
    door.outside[2] += dz;
    review.push({
      asset: id,
      door: door.id,
      ramp: node,
      outsideBefore: before,
      outsideAfter: [...door.outside],
    });
  }
  for (const room of gameplay.interiors)
    for (const door of room.doors) {
      assert.equal(door.passageStructure, undefined);
      door.passageStructure = { volumes: [door.node], blockers: [] };
    }
  // Clear only authored top surfaces from this asset's own support volumes.
  // This keeps rounded caps walkable while leaving building bodies and foreign walls solid.
  gameplay.movementClearances ??= [];
  for (const surface of gameplay.surfaces) {
    const id = `${surface.id}-support-clearance`;
    assert.ok(!gameplay.movementClearances.some((entry) => entry.id === id));
    gameplay.movementClearances.push({
      id,
      node: surface.node,
      polygon: structuredClone(surface.polygon),
      height: structuredClone(surface.height),
      holes: structuredClone(surface.holes ?? []),
    });
  }
  validateAssetGameplay(gameplay, asset);
  const ref = document.assetSources.find((entry) => entry.id === id);
  changed.push({
    asset: id,
    descriptorSha256: ref.descriptor_sha256,
    modelSha256: ref.model_sha256,
    gameplay,
  });
  const room = gameplay.interiors[0];
  const part = document.objects.find((object) => object.node === `asset:${id}:${room.node}`);
  const shape = asset.parts.find((entry) => entry.node === room.node).obstacle_local_game;
  const matrix = partMatrix(document.camera, document, { ...part, obstacle: shape });
  const translation = sceneToGame(document.camera, matrix.slice(12, 15));
  fixtures.push({
    map: "york",
    assets: [id],
    size: [3200, 3200],
    copyXs: [800.25, 2400.25],
    assemblyY: 1800.75,
    terrainHeight: minimums[0] + translation[2],
    tracePassages: true,
    entrances: room.doors.map((door, index) => {
      return {
        asset: id,
        door: door.id,
        sourceHeight: minimums[0],
        sourceLocal: groundApproaches[index],
      };
    }),
  });
}
const terrain = assets.get("york-west-town-raised-terrain");
const gameplay = structuredClone(terrain.gameplay);
assert.equal(gameplay.surfaces.length, 1);
assert.ok([undefined, true].includes(gameplay.surfaces[0].acceptsNavigationJoins));
gameplay.surfaces[0].acceptsNavigationJoins = true;
validateAssetGameplay(gameplay, terrain);
const ref = document.assetSources.find((entry) => entry.id === terrain.id);
if (terrain.gameplay.surfaces[0].acceptsNavigationJoins !== true)
  changed.push({
    asset: terrain.id,
    descriptorSha256: ref.descriptor_sha256,
    modelSha256: ref.model_sha256,
    gameplay,
  });
const output = await fs.mkdtemp("work/needs-review/map-compile/york-entrance-ramps-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(changed));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({ scope: "unpublished-ramp-floor-candidate", review }),
);
for (const fixture of fixtures)
  await fs.writeFile(`${output}/${fixture.assets[0]}.json`, JSON.stringify(fixture));
console.log(output);
