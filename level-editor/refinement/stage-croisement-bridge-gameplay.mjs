// Asset-only deck candidate. Rail/pier collision and native route review must
// be completed before publication; this script never reads level game data.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { sceneToGame, applyAffineMatrix, signedPolygonArea } from "../shared/src/geometry.ts";
import { gameToScene } from "../shared/src/scene.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";

const id = "croisement03-timber-bridge";
const solidDeck = process.argv.includes("--solid-deck");
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
const underside = [];
for (let face = 0; face < indices.length / 3; face++) {
  const points = indices.slice(face * 3, face * 3 + 3).map((i) => vertices[i]);
  const [a, b, c] = points;
  const area = ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])) / 2;
  // The deck is the pair of large upward-facing, nearly horizontal faces.
  // Export rounding leaves a few ten-thousandths of height variation.
  if (
    Math.abs(area) > 9000 &&
    Math.max(...points.map((p) => p[2])) - Math.min(...points.map((p) => p[2])) < 0.001
  )
    (area > 0 ? faces : underside).push({
      face,
      points: points.map((p) => sceneToGame(camera, p)),
    });
}
assert.equal(faces.length, 2, "Review changed deck topology");
assert.equal(underside.length, 2, "Review changed deck underside topology");
const ground = faces.flatMap((face) => face.points).reduce((sum, p) => sum + p[2], 0) / 6;
descriptor.gameplay = {
  version: 1,
  collision: "none",
  placementGroundHeight: ground,
  volumes: faces.map(({ face, points }) => ({
    id: `deck-body-${face}`,
    node: node.name,
    movementHeadroom: 80,
    shape: {
      points: points.map(([x, y, z_top]) => {
        const matches = underside
          .flatMap((face) => face.points)
          .filter((p) => Math.hypot(p[0] - x, p[1] - y) < 0.001);
        assert.ok(matches.length > 0, "Deck underside must match its top perimeter");
        const z_bottom = Math.min(...matches.map((p) => p[2]));
        assert.ok(z_top > z_bottom);
        return { x, y, z_top, z_bottom };
      }),
      solid: true,
      opaque: true,
      mouse: true,
      show_shadow_polygon: true,
      default_material: 1,
    },
  })),
  surfaces: faces.map(({ face, points }) => ({
    id: `deck-face-${face}`,
    node: node.name,
    polygon: points.map(([x, y]) => [x, y]),
    height: points.map((p) => p[2]),
    navigationRegion: "bridge-deck",
    preserveMovementPrecision: true,
    projectionMaterials: { defaultMaterial: 1, regions: [] },
    navigationJoins: points.flatMap((a, i) => {
      const b = points[(i + 1) % 3];
      const sceneLength = Math.hypot(b[0] - a[0], (b[1] - a[1]) / Math.sin((35 * Math.PI) / 180));
      return sceneLength < 130 ? [[a, b]] : [];
    }),
    navigationJoinMinimumOverlap: 12,
    navigationJoinHeightTolerance: 0.001,
  })),
  doors: [],
  draft: {
    issues: [
      "Bridge deck candidate only: rail/pier collision, receiving seams and rendered actor occlusion still require authoring and native route review.",
    ],
  },
};
if (!solidDeck) delete descriptor.gameplay.volumes;
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
    const matrix = partMatrix(camera, document, document.objects[0]);
    const world = (p) => sceneToGame(camera, applyAffineMatrix(matrix, gameToScene(camera, ...p)));
    const ends = descriptor.gameplay.surfaces.flatMap((surface) => surface.navigationJoins);
    const centers = ends.map(([a, b]) => a.map((n, i) => (n + b[i]) / 2));
    const delta = centers[1].map((n, i) => n - centers[0][i]);
    const length = Math.hypot(delta[0], delta[1]);
    const terrain = { version: 1, spacing: 100, vertices: [], cells: [] };
    const endpoints = [];
    for (const [i, [a, b]] of ends.entries()) {
      const extend = (p, distance) =>
        p.map((n, axis) =>
          axis === 2 ? n : n + (((i ? 1 : -1) * delta[axis]) / length) * distance,
        );
      let points = [a, b, extend(b, 80), extend(a, 80)].map(world);
      if (signedPolygonArea(points.map((p) => p.slice(0, 2))) < 0) points.reverse();
      const offset = terrain.vertices.length;
      terrain.vertices.push(...points.map((position, j) => ({ id: `bank-${i}-${j}`, position })));
      terrain.cells.push({
        id: `bank-${i}`,
        material: document.terrain.cells[0].material,
        vertices: [0, 1, 2, 3].map((j) => offset + j),
      });
      const [x, y, z] = world(extend(centers[i], 40));
      endpoints.push([x, y - z]);
    }
    const gapDocument = { ...document, terrain };
    results.at(-1).routes = [endpoints];
    const gap = compileMap(gapDocument, [0, 0, 1000, 1000], new Map([[id, descriptor]]), {
      bestEffort: false,
    });
    const banks = receiverAreas(gap, 3);
    const shared = [...receiverAreas(gap, 1)].filter((area) => banks.has(area));
    assert.equal(shared.length, 1, "Both banks and deck must join across the void");
    const without = compileMap(
      { ...gapDocument, objects: [], groups: [], assetSources: [] },
      [0, 0, 1000, 1000],
      new Map(),
      { bestEffort: false },
    );
    assert.equal(
      without.descriptor.asset_geometry.motion_data.layers.flat().length,
      2,
      "Banks alone must remain disconnected",
    );
    const gapFile = file.replace(".level.json", "-gap.level.json");
    await fs.writeFile(`${output}/${gapFile}`, JSON.stringify(gap.descriptor));
    await fs.writeFile(`${output}/${gapFile}.scene.json`, JSON.stringify(gapDocument));
    const [sector, layer] = JSON.parse(shared[0]);
    const ray_probes = (descriptor.gameplay.volumes ?? []).flatMap((volume) => {
      const points = volume.shape.points;
      const mean = (key) => points.reduce((sum, p) => sum + p[key], 0) / points.length;
      const x = mean("x"),
        y = mean("y"),
        top = mean("z_top"),
        bottom = mean("z_bottom");
      return [
        {
          name: `${volume.id}-through`,
          clear: false,
          endpoints: [
            [x, y, bottom - 5],
            [x, y, top + 5],
          ].map(world),
        },
        {
          name: `${volume.id}-above`,
          clear: true,
          endpoints: [
            [x - 2, y, top + 5],
            [x + 2, y, top + 5],
          ].map(world),
        },
        {
          name: `${volume.id}-below`,
          clear: true,
          endpoints: [
            [x - 2, y, bottom - 5],
            [x + 2, y, bottom - 5],
          ].map(world),
        },
      ];
    });
    results.push({
      file: gapFile,
      map: gapFile,
      rotation,
      elevation,
      layer,
      sector,
      routes: [endpoints],
      ray_probes,
      banksDisconnectedWithoutBridge: true,
    });
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
    solidDeck,
    scope: "Unpublished deck construction only; collision and connectivity incomplete",
  }),
);
await fs.writeFile(
  `${output}/diagnostics.json`,
  JSON.stringify({ complete: true, scope: "static-geometry-only-not-gameplay-parity", results }),
);
console.log(JSON.stringify({ output, exports: results.length, ground }));
