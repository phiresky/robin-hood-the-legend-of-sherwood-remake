// Stage the terrace's physical receiving plane as a reusable walking surface.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { sceneToGame } from "../shared/src/geometry.ts";

const flags = process.argv.slice(2);
assert.ok(flags.every((flag) => flag === "--ground-contact"));
const groundContact = flags.includes("--ground-contact");
const document = await readStoredMap("library/scenes/york.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
const id = "york-south-town-raised-terrace";
const asset = assets.get(id);
assert.ok(asset);
const gameplay = structuredClone(asset.gameplay);
assert.equal(gameplay.surfaces.length, 0);
assert.equal(gameplay.projectionReceivers.length, 1);
const receiver = gameplay.projectionReceivers[0];
const part = asset.parts.find((part) => part.node === receiver.volume);
assert.ok(part?.obstacle_local_game.solid);
const shape = part.obstacle_local_game;
const vertices = shape.points.map((point) => [point.x, point.y, point.z_top]);
const plane = heightPlane(vertices.slice(0, 3));
const residual = Math.max(
  ...vertices.map((point) => Math.abs(planeHeight(plane, point) - point[2])),
);
assert.ok(residual < 0.002, "Only normalize subpixel receiving-plane quantization");
const polygon = vertices.map(([x, y]) => [x, y]);
let contactCorrection = 0;
if (groundContact) {
  assert.ok(shape.points.every((point) => point.z_bottom === 0));
  assert.ok(vertices[0][2] < 0.25 && vertices[1][2] < 0.25);
  assert.ok(vertices.slice(2).every((point) => point[2] > 1));
  const gradientSquared = plane[0] ** 2 + plane[1] ** 2;
  assert.ok(gradientSquared > 1e-6);
  // Extend just the low edge onto the same plane's ground intersection.
  // This is a reviewed asset boundary correction, not a compiler tolerance.
  for (const index of [0, 1]) {
    const point = polygon[index],
      z = planeHeight(plane, point);
    const delta = [plane[0], plane[1]].map((value) => (-value * z) / gradientSquared);
    contactCorrection = Math.max(contactCorrection, Math.hypot(...delta));
    polygon[index] = point.map((value, axis) => value + delta[axis]);
  }
  assert.ok(contactCorrection < 1, "Ground contact must remain within one map unit");
}
const height = polygon.map((point) => planeHeight(plane, point));
assert.equal(shape.material_indices.length, 0);
gameplay.surfaces = [
  {
    id: `${part.node}-physical-terrace`,
    node: part.node,
    polygon,
    height,
    navigationRegion: `${part.node}-physical-terrace`,
    preserveMovementPrecision: true,
    navigationJoins: polygon.map((point, index) => {
      const next = (index + 1) % polygon.length;
      return [
        [...point, height[index]],
        [...polygon[next], height[next]],
      ];
    }),
    projectionMaterials: {
      defaultMaterial: shape.default_material,
      regions: [],
      planePoints: vertices.slice(0, 3),
    },
  },
];
gameplay.projectionReceivers = [];
assert.equal(gameplay.movementSolids, undefined);
assert.equal(gameplay.movementClearances, undefined);
gameplay.movementSolids = [part.node];
if (groundContact) {
  const volumeId = `${part.node}-ground-contact-solid`;
  assert.ok(!gameplay.volumes?.some((volume) => volume.id === volumeId));
  gameplay.volumes ??= [];
  gameplay.volumes.push({
    id: volumeId,
    node: part.node,
    shape: {
      solid: true,
      opaque: false,
      mouse: false,
      show_shadow_polygon: false,
      default_material: shape.default_material,
      points: polygon.map(([x, y], index) => ({
        x,
        y,
        z_bottom: 0,
        z_top: Math.max(0, height[index]),
      })),
    },
  });
  gameplay.movementSolids = [volumeId];
}
gameplay.movementClearances = [
  {
    id: `${part.node}-support-clearance`,
    node: part.node,
    polygon,
    height,
  },
];
validateAssetGameplay(gameplay, asset);
const ref = document.assetSources.find((ref) => ref.id === id);
const output = await fs.mkdtemp("work/needs-review/map-compile/york-south-town-floor-");
await fs.writeFile(
  `${output}/edits.json`,
  JSON.stringify([
    {
      asset: id,
      descriptorSha256: ref.descriptor_sha256,
      modelSha256: ref.model_sha256,
      gameplay,
    },
  ]),
);
await fs.writeFile(`${output}/scene.json`, JSON.stringify(document));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({
    scope: "unpublished-physical-terrace-candidate-not-parity-proof",
    plane,
    residual,
    groundContact,
    contactCorrection,
    unchangedEntranceOwner: "york-south-wall-lane-square-stone-tower",
  }),
);
const tower = assets.get("york-south-wall-lane-square-stone-tower");
const door = tower.gameplay.interiors[0].doors[0];
const direction = door.outside.slice(0, 2).map((value, axis) => value - door.middle[axis]);
const length = Math.hypot(...direction);
assert.ok(length > 0);
const translation = (asset, node) => {
  const objects = document.objects.filter((object) => object.node === `asset:${asset.id}:${node}`);
  assert.equal(objects.length, 1);
  const shape = asset.parts.find((part) => part.node === node).obstacle_local_game;
  const matrix = partMatrix(document.camera, document, { ...objects[0], obstacle: shape });
  for (let i = 0; i < 16; i++) {
    if ([12, 13, 14].includes(i)) continue;
    assert.ok(Math.abs(matrix[i] - (i % 5 === 0 ? 1 : 0)) < 1e-8);
  }
  return sceneToGame(document.camera, matrix.slice(12, 15));
};
const towerOrigin = translation(tower, door.node),
  terraceOrigin = translation(asset, part.node);
const sourceLocal = direction.map((value, axis) => door.outside[axis] + (value * 64) / length);
const sourceHeight =
  planeHeight(
    plane,
    sourceLocal.map((value, axis) => value + towerOrigin[axis] - terraceOrigin[axis]),
  ) +
  terraceOrigin[2] -
  towerOrigin[2];
assert.ok(
  Math.abs(
    planeHeight(
      plane,
      door.outside
        .slice(0, 2)
        .map((value, axis) => value + towerOrigin[axis] - terraceOrigin[axis]),
    ) +
      terraceOrigin[2] -
      towerOrigin[2] -
      door.outside[2],
  ) < 1e-6,
  "Entrance must lie on the authored terrace plane",
);
const fixture = {
  map: "york",
  assets: [tower.id, id],
  size: [5000, 4000],
  copyXs: [1200.25, 3600.25],
  assemblyY: 2300.75,
  terrainHeight: 0,
  tracePassages: true,
  entrances: [{ asset: tower.id, door: door.id, sourceLocal, sourceHeight }],
};
await fs.writeFile(`${output}/terrace-tower.json`, JSON.stringify(fixture));
if (groundContact) {
  const [a, b] = polygon;
  const length = Math.hypot(b[0] - a[0], b[1] - a[1]);
  const outward = [(b[1] - a[1]) / length, (a[0] - b[0]) / length];
  const sourceLocal = [0, 1].map(
    (axis) =>
      (a[axis] + b[axis]) / 2 + outward[axis] * 32 + terraceOrigin[axis] - towerOrigin[axis],
  );
  fixture.entrances = [
    {
      asset: tower.id,
      door: door.id,
      sourceLocal,
      sourceHeight: terraceOrigin[2] - towerOrigin[2],
      sourceOnTerrain: true,
    },
  ];
  await fs.writeFile(`${output}/flat-ground.json`, JSON.stringify(fixture));
}
console.log(output);
