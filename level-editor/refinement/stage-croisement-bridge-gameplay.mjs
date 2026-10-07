// Asset-only deck candidate. Rail/pier collision and native route review must
// be completed before publication; this script never reads level game data.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { sceneToGame } from "../shared/src/geometry.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";

const id = "croisement03-timber-bridge";
const camera = { kind: "oblique-orthographic", elevation_deg: 35 };
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json")).assets;
const entry = index.find((asset) => asset.id === id);
assert.ok(entry);
const descriptorBytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
assert.equal(hash(descriptorBytes), entry.descriptor_sha256);
const descriptor = JSON.parse(descriptorBytes);
assert.equal(descriptor.gameplay, undefined);
const bytes = await fs.readFile(`library/3d-assets/${entry.model}`);
const length = bytes.readUInt32LE(12);
const gltf = JSON.parse(bytes.subarray(20, 20 + length));
const binary = bytes.subarray(28 + length);
const node = gltf.nodes.find((node) => node.name === descriptor.parts[0].node);
assert.equal(node.children.length, 1);
const child = gltf.nodes[node.children[0]];
for (const part of [node, child])
  for (const key of ["matrix", "translation", "rotation", "scale"])
    assert.equal(part[key], undefined, `Review changed local mesh frame: ${key}`);
const primitives = gltf.meshes[child.mesh].primitives;
assert.equal(primitives.length, 1);
function accessor(index) {
  const a = gltf.accessors[index],
    view = gltf.bufferViews[a.bufferView];
  assert.equal(a.sparse, undefined);
  const width = { SCALAR: 1, VEC3: 3 }[a.type];
  const size = { 5123: 2, 5125: 4, 5126: 4 }[a.componentType];
  const read = { 5123: "readUInt16LE", 5125: "readUInt32LE", 5126: "readFloatLE" }[a.componentType];
  assert.ok(width && size && read);
  return Array.from({ length: a.count }, (_, i) =>
    Array.from({ length: width }, (_, j) =>
      binary[read](
        (view.byteOffset ?? 0) +
          (a.byteOffset ?? 0) +
          i * (view.byteStride ?? width * size) +
          j * size,
      ),
    ),
  );
}
const primitive = primitives[0];
assert.equal(primitive.mode ?? 4, 4);
const vertices = accessor(primitive.attributes.POSITION);
const indices = accessor(primitive.indices).flat();
const faces = [];
for (let face = 0; face < indices.length / 3; face++) {
  const points = indices.slice(face * 3, face * 3 + 3).map((i) => vertices[i]);
  const [a, b, c] = points;
  const area = ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])) / 2;
  // The deck is the pair of large upward-facing, nearly horizontal faces.
  // Export rounding leaves a few ten-thousandths of height variation.
  if (
    area > 9000 &&
    Math.max(...points.map((p) => p[2])) - Math.min(...points.map((p) => p[2])) < 0.001
  )
    faces.push({ face, points: points.map((p) => sceneToGame(camera, p)) });
}
assert.equal(faces.length, 2, "Review changed deck topology");
const ground = faces.flatMap((face) => face.points).reduce((sum, p) => sum + p[2], 0) / 6;
descriptor.gameplay = {
  version: 1,
  collision: "none",
  placementGroundHeight: ground,
  surfaces: faces.map(({ face, points }) => ({
    id: `deck-face-${face}`,
    node: node.name,
    polygon: points.map(([x, y]) => [x, y]),
    height: points.map((p) => p[2]),
    navigationRegion: "bridge-deck",
    preserveMovementPrecision: true,
    navigationJoins: points.flatMap((a, i) => {
      const b = points[(i + 1) % 3];
      const sceneLength = Math.hypot(b[0] - a[0], (b[1] - a[1]) / Math.sin((35 * Math.PI) / 180));
      return sceneLength < 130 ? [[a, b]] : [];
    }),
    navigationJoinMinimumOverlap: 12,
    navigationJoinHeightTolerance: 0.001,
    projectionMaterials: { defaultMaterial: 1, regions: [] },
  })),
  doors: [],
  draft: {
    issues: [
      "Bridge deck candidate only: rail/pier collision, receiving seams and rendered actor occlusion still require authoring and native route review.",
    ],
  },
};
assert.equal(descriptor.gameplay.surfaces.flatMap((surface) => surface.navigationJoins).length, 2);
const reference = {
  id,
  role: "objects",
  model: `3d-assets/${entry.model}`,
  model_sha256: hash(bytes),
  descriptor: `3d-assets/${entry.descriptor}`,
  descriptor_sha256: hash(descriptorBytes),
  model_scene: entry.model_scene,
  resources: descriptor.resources ?? [],
};
const output = await fs.mkdtemp("work/map-compile/croisement-bridge-gameplay-");
const results = [];
const receiverAreas = (compiled, material) =>
  new Set(
    compiled.descriptor.asset_geometry.sight_obstacles
      .filter(
        (obstacle) =>
          Array.isArray(obstacle.projection_area) && obstacle.default_material === material,
      )
      .map((obstacle) => JSON.stringify(obstacle.projection_area)),
  );
for (const elevation of [0, 40])
  for (const rotation of [0, 37, 90, 180, 270]) {
    const empty = {
      version: 1,
      map: id,
      camera,
      size: [1000, 1000],
      objects: [],
      groups: [],
      sceneAssets: [],
      assetSources: [],
      terrain: createTerrainGrid([0, 0, 1000, 1000], 250, elevation),
    };
    const { document } = insertProjectionAsset(empty, descriptor, reference, [
      500.25,
      500.75,
      elevation,
    ]);
    document.groups[0].transform.rot_deg = rotation;
    const compiled = compileMap(document, [0, 0, 1000, 1000], new Map([[id, descriptor]]), {
      bestEffort: false,
    });
    assert.ok(compiled.descriptor.asset_geometry.motion_data.layers.flat().length);
    const groundAreas = receiverAreas(compiled, 3);
    assert.equal(
      [...receiverAreas(compiled, 1)].filter((area) => groundAreas.has(area)).length,
      1,
      "Deck ends must connect to matching terrain",
    );
    for (const delta of [-1, 1]) {
      const shifted = structuredClone(document);
      shifted.terrain = createTerrainGrid([0, 0, 1000, 1000], 250, elevation + delta);
      const rejected = compileMap(shifted, [0, 0, 1000, 1000], new Map([[id, descriptor]]), {
        bestEffort: false,
      });
      const terrainAreas = receiverAreas(rejected, 3);
      const woodAreas = receiverAreas(rejected, 1);
      assert.ok(woodAreas.size > 0 && terrainAreas.size > 0);
      assert.ok(
        [...woodAreas].every((area) => !terrainAreas.has(area)),
        "Wrong-height terrain must not connect to the deck",
      );
    }
    const file = `${id}-${elevation}-${rotation}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
    results.push({ file, map: file, rotation, elevation, mismatchedLandingsRejected: 2 });
  }
await fs.writeFile(
  `${output}/edits.json`,
  JSON.stringify([
    { asset: id, descriptorSha256: hash(descriptorBytes), gameplay: descriptor.gameplay },
  ]),
);
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({
    modelSha256: hash(bytes),
    faces: faces.map((f) => f.face),
    ground,
    scope: "Unpublished deck construction only; collision and connectivity incomplete",
  }),
);
await fs.writeFile(
  `${output}/diagnostics.json`,
  JSON.stringify({ complete: true, scope: "static-geometry-only-not-gameplay-parity", results }),
);
console.log(JSON.stringify({ output, exports: results.length, ground }));
