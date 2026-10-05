// Run from level-editor; add --supports after author-imported-bridge-supports.py.
// Writes unpublished candidates and native-test fixtures under work/map-compile.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { createRequire } from "node:module";
import { sceneToGame, signedPolygonArea, applyAffineMatrix } from "../shared/src/geometry.ts";
import { gameToScene } from "../shared/src/scene.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";

const base = "library/3d-assets/sketchfab/sketchfab-long-wood-bridge";
const descriptorBytes = await fs.readFile(`${base}/asset.json`);
const descriptor = JSON.parse(descriptorBytes);
const published = process.argv.includes("--published");
const publishedGameplay = structuredClone(descriptor.gameplay);
const bytes = await fs.readFile(`${base}/model.glb`);
const review = JSON.parse(
  await fs.readFile("refinement/catalogs/sketchfab-long-wood-bridge-deck-review.json", "utf8"),
);
assert.equal(
  createHash("sha256").update(descriptorBytes).digest("hex"),
  published ? review.publishedDescriptorSha256 : review.descriptorSha256,
);
if (published) assert.ok(process.argv.includes("--structure"));
assert.equal(createHash("sha256").update(bytes).digest("hex"), review.modelSha256);
const jsonLength = bytes.readUInt32LE(12);
const gltf = JSON.parse(bytes.subarray(20, 20 + jsonLength).toString());
const binary = bytes.subarray(28 + jsonLength);
function accessor(index) {
  const a = gltf.accessors[index],
    view = gltf.bufferViews[a.bufferView];
  const width = { SCALAR: 1, VEC3: 3 }[a.type];
  assert.ok(width);
  const size = { 5123: 2, 5125: 4, 5126: 4 }[a.componentType];
  assert.ok(size);
  const read = { 5123: "readUInt16LE", 5125: "readUInt32LE", 5126: "readFloatLE" }[a.componentType];
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
const node = gltf.nodes.find((n) => n.name === "scenery-long-wood-bridge");
assert.ok(node);
assert.equal(node.matrix, undefined);
assert.equal(node.translation, undefined);
assert.equal(node.rotation, undefined);
assert.equal(node.scale, undefined);
const [primitive] = gltf.meshes[node.mesh].primitives;
assert.equal(primitive.mode ?? 4, 4);
const vertices = accessor(primitive.attributes.POSITION),
  indices = accessor(primitive.indices).flat();
assert.equal(vertices.length, 2388);
assert.equal(indices.length, 2708 * 3);
const camera = { kind: "oblique-orthographic", elevation_deg: 35 };
// Reviewed top-facing deck triangles; under-deck beams and rails are excluded.
const faces = review.triangleIndices;
const surfaces = faces.map((face) => {
  const scene = indices.slice(face * 3, face * 3 + 3).map((i) => vertices[i]);
  assert.ok(scene.every((p) => p[2] > 87 && p[2] < 95 && Math.abs(p[0]) < 31));
  let points = scene.map((p) => sceneToGame(camera, p));
  if (signedPolygonArea(points.map((p) => p.slice(0, 2))) < 0) points.reverse();
  const joins = points.flatMap((a, i) => {
    const b = points[(i + 1) % 3];
    return Math.abs(Math.abs(a[1]) - 168.42010498046875 * Math.sin((35 * Math.PI) / 180)) < 1e-6 &&
      Math.abs(a[1] - b[1]) < 1e-6
      ? [[a, b]]
      : [];
  });
  return {
    id: `deck-face-${face}`,
    node: node.name,
    polygon: points.map((p) => p.slice(0, 2)),
    height: points.map((p) => p[2]),
    navigationRegion: "bridge-deck",
    preserveMovementPrecision: true,
    projectionMaterials: { defaultMaterial: 1, regions: [] },
    ...(joins.length
      ? {
          navigationJoins: joins,
          navigationJoinMinimumOverlap: 12,
          navigationJoinHeightTolerance: 0.01,
        }
      : {}),
  };
});
assert.equal(surfaces.flatMap((s) => s.navigationJoins ?? []).length, 2);
descriptor.gameplay = {
  version: 1,
  collision: "none",
  surfaces,
  doors: [],
  draft: {
    issues: [
      "Unpublished deck-only authoring candidate: support, railing, projectile and sight collision remain unauthored; under-bridge clearance is not verified.",
    ],
  },
};
let supportHulls;
if (process.argv.includes("--supports") || process.argv.includes("--structure")) {
  const require = createRequire(new URL("../pipeline/package.json", import.meta.url));
  const clipping = require("polygon-clipping");
  supportHulls = JSON.parse(
    await fs.readFile("work/map-compile/imported-bridge-support-hulls.json"),
  );
  assert.equal(supportHulls.modelSha256, review.modelSha256);
  const structure = process.argv.includes("--structure")
    ? JSON.parse(await fs.readFile("work/map-compile/imported-bridge-structure-hulls.json"))
    : undefined;
  if (structure) assert.equal(structure.modelSha256, review.modelSha256);
  descriptor.gameplay.volumes = [];
  for (const support of [...supportHulls.supports, ...(structure?.supports ?? [])]) {
    let totalVolume = 0;
    const mergeFaces = (faces) => {
      const groups = [];
      for (const face of faces) {
        let group = groups.find((g) => g.plane.every((n, i) => Math.abs(n - face.plane[i]) < 1e-8));
        if (!group) {
          group = { plane: face.plane, polygons: [] };
          groups.push(group);
        }
        group.polygons.push([face.indices.map((i) => support.vertices[i].slice(0, 2))]);
      }
      return groups.flatMap((group) =>
        clipping.union(...group.polygons).map((polygon) => {
          assert.equal(polygon.length, 1);
          return { plane: group.plane, ring: polygon[0] };
        }),
      );
    };
    const upper = mergeFaces(support.faces.filter((f) => f.plane[2] > 1e-8)),
      lower = mergeFaces(support.faces.filter((f) => f.plane[2] < -1e-8));
    const ring = (f) => f.ring;
    const height = (f, [x, y]) => -(f.plane[0] * x + f.plane[1] * y + f.plane[3]) / f.plane[2];
    let piece = 0;
    for (const top of upper)
      for (const bottom of lower)
        for (const polygon of clipping.intersection([ring(top)], [ring(bottom)])) {
          assert.equal(polygon.length, 1);
          const points = polygon[0].slice(0, -1);
          // Coplanar box faces can intersect in numerical dust. Keep cells
          // large enough to define a height plane; volume conservation below
          // still bounds the amount discarded for each complete hull.
          if (Math.abs(signedPolygonArea(points)) < 1e-7) continue;
          const shapePoints = points.map((p) => {
            const low = height(bottom, p),
              high = height(top, p);
            assert.ok(high >= low - 1e-7);
            return { x: p[0], y: p[1], z_bottom: low, z_top: Math.max(low, high) };
          });
          for (let i = 1; i + 1 < points.length; i++) {
            const tri = [0, i, i + 1];
            totalVolume +=
              (Math.abs(signedPolygonArea(tri.map((j) => points[j]))) *
                tri.reduce((sum, j) => sum + shapePoints[j].z_top - shapePoints[j].z_bottom, 0)) /
              3;
          }
          descriptor.gameplay.volumes.push({
            id: `${support.id}-piece-${piece++}`,
            node: node.name,
            shape: {
              points: shapePoints,
              solid: true,
              opaque: true,
              mouse: true,
              show_shadow_polygon: true,
              default_material: 1,
            },
          });
        }
    assert.ok(
      Math.abs(totalVolume - support.volume) < support.volume * 1e-7,
      `Partition changed support volume: ${support.id}`,
    );
  }
  for (const thickness of structure?.deckThickness ?? []) {
    const surface = surfaces.find((s) => s.id === `deck-face-${thickness.face}`);
    assert.ok(surface);
    const triangle = indices
      .slice(thickness.face * 3, thickness.face * 3 + 3)
      .map((i) => sceneToGame(camera, vertices[i]));
    descriptor.gameplay.volumes.push({
      id: `deck-body-${thickness.face}`,
      node: node.name,
      shape: {
        points: surface.polygon.map(([x, y], i) => {
          const source = triangle.findIndex((p) => Math.hypot(p[0] - x, p[1] - y) < 1e-7);
          assert.ok(source >= 0);
          return { x, y, z_bottom: thickness.bottomHeights[source], z_top: surface.height[i] };
        }),
        solid: true,
        opaque: true,
        mouse: true,
        show_shadow_polygon: true,
        default_material: 1,
      },
    });
  }
  if (structure) {
    for (const volume of descriptor.gameplay.volumes)
      volume.movementHeadroom = review.structureAuthoring.movementHeadroom;
  }
  descriptor.gameplay.draft.issues = [
    structure
      ? "Unpublished bridge candidate: fitted wood collision and authored upright headroom require visual review."
      : "Unpublished bridge candidate: railing, deck-body and brace collision remain unauthored; support hulls and underpass routes are under review.",
  ];
}
validateAssetGameplay(descriptor.gameplay, descriptor);
if (published) {
  const withoutDraft = (gameplay) => {
    // Compare serialized definitions, where JSON normalizes negative zero.
    const normalized = JSON.parse(JSON.stringify(gameplay));
    delete normalized.draft;
    return normalized;
  };
  assert.deepEqual(withoutDraft(descriptor.gameplay), withoutDraft(publishedGameplay));
  descriptor.gameplay = publishedGameplay;
}
const output = await fs.mkdtemp("work/map-compile/imported-bridge-deck-");
await fs.writeFile(
  `${output}/candidate.gameplay.json`,
  JSON.stringify(descriptor.gameplay, null, 2),
);
await fs.writeFile(
  `${output}/source.json`,
  JSON.stringify(
    {
      descriptorSha256: createHash("sha256").update(descriptorBytes).digest("hex"),
      modelSha256: createHash("sha256").update(bytes).digest("hex"),
      deckFaces: faces,
    },
    null,
    2,
  ),
);
const reference = {
  id: descriptor.id,
  descriptor: base.slice(8) + "/asset.json",
  descriptor_sha256: createHash("sha256").update(descriptorBytes).digest("hex"),
  model: base.slice(8) + "/model.glb",
  model_sha256: createHash("sha256").update(bytes).digest("hex"),
  model_scene: "default",
  resources: [],
};
const endHeight = surfaces.flatMap((s) => s.navigationJoins ?? [])[0][0][2];
const results = [];
const landingChecks = [];
const receiverAreas = (result, material) =>
  new Set(
    result.descriptor.asset_geometry.sight_obstacles
      .filter(
        (obstacle) =>
          Array.isArray(obstacle.projection_area) && obstacle.default_material === material,
      )
      .map((obstacle) => JSON.stringify(obstacle.projection_area)),
  );
for (const rotation of [0, 37, 90, 180, 270]) {
  const empty = {
    version: 1,
    map: "Imported bridge deck candidate",
    camera,
    size: [1000, 1000],
    objects: [],
    groups: [],
    sceneAssets: [],
    assetSources: [],
    terrain: createTerrainGrid([0, 0, 1000, 1000], 250, endHeight),
  };
  const { document } = insertProjectionAsset(empty, descriptor, reference, [500, 500, endHeight]);
  document.groups[0].transform.rot_deg = rotation;
  const compiled = compileMap(
    document,
    [0, 0, 1000, 1000],
    new Map([[descriptor.id, descriptor]]),
    { bestEffort: false },
  );
  const areaCount = (result) =>
    result.descriptor.asset_geometry.motion_data.layers.reduce((n, layer) => n + layer.length, 0);
  const file = `bridge-${rotation}.level.json`;
  await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
  const groundAreas = receiverAreas(compiled, 3);
  const sharedAreas = [...receiverAreas(compiled, 1)].filter((area) => groundAreas.has(area));
  assert.equal(
    sharedAreas.length,
    1,
    "Matching landings and deck must share one navigation region",
  );
  const [routeSector, routeLayer] = JSON.parse(sharedAreas[0]);
  for (const offset of [-1, 1]) {
    const mismatched = structuredClone(document);
    mismatched.terrain = createTerrainGrid([0, 0, 1000, 1000], 250, endHeight + offset);
    const rejected = compileMap(
      mismatched,
      [0, 0, 1000, 1000],
      new Map([[descriptor.id, descriptor]]),
      { bestEffort: false },
    );
    const woodAreas = receiverAreas(rejected, 1),
      groundAreas = receiverAreas(rejected, 3);
    assert.ok(woodAreas.size > 0 && groundAreas.size > 0);
    assert.ok(
      [...woodAreas].every((area) => !groundAreas.has(area)),
      "A mismatched landing must not share navigation with the deck",
    );
    landingChecks.push({ rotation, heightOffset: offset, navigationRegions: areaCount(rejected) });
  }
  const matrix = partMatrix(camera, document, document.objects[0]);
  const project = (p) => {
    const [x, y, z] = sceneToGame(camera, applyAffineMatrix(matrix, gameToScene(camera, ...p)));
    return [x, y - z];
  };
  const ends = surfaces
    .flatMap((s) => s.navigationJoins ?? [])
    .map(([a, b]) => a.map((n, i) => (n + b[i]) / 2));
  const direction = ends[1].map((n, i) => n - ends[0][i]);
  const length = Math.hypot(direction[0], direction[1]);
  const outward = ends.map((p, i) =>
    p.map((n, j) => (j === 2 ? n : n + (((i === 0 ? -1 : 1) * direction[j]) / length) * 24)),
  );
  results.push({
    file,
    map: file,
    layer: routeLayer,
    sector: routeSector,
    navigationRegions: areaCount(compiled),
    warnings: compiled.warnings,
    routes: [[project(outward[0]), project(outward[1])]],
  });
  if (supportHulls) {
    const underpass = structuredClone(document);
    underpass.terrain = createTerrainGrid([0, 0, 1000, 1000], 250, 0);
    const lower = compileMap(
      underpass,
      [0, 0, 1000, 1000],
      new Map([[descriptor.id, descriptor]]),
      { bestEffort: false },
    );
    const lowerFile = `underpass-${rotation}.level.json`;
    await fs.writeFile(`${output}/${lowerFile}`, JSON.stringify(lower.descriptor));
    let largest,
      sectorIndex = 0;
    lower.descriptor.asset_geometry.motion_data.layers.forEach((regions, layer) =>
      regions.forEach((region) => {
        const area = Math.abs(signedPolygonArea(region.polygon.points));
        if (!largest || area > largest.area) largest = { layer, sector: sectorIndex, area };
        sectorIndex++;
      }),
    );
    const blocked = supportHulls.supports.map((s) => {
      const foot = s.vertices.filter((v) => v[2] === 0);
      assert.ok(foot.length >= 3);
      return project([
        foot.reduce((n, p) => n + p[0], 0) / foot.length,
        foot.reduce((n, p) => n + p[1], 0) / foot.length,
        0,
      ]);
    });
    const headroom = process.argv.includes("--structure");
    if (headroom) blocked.push(project([0, 0, 0]), project([0, -14, 0]), project([0, 14, 0]));
    // Keep endpoints inside a terrain triangle: the native route helper checks
    // receiver identity, which is ambiguous exactly on a shared triangle edge.
    results.push({
      file: lowerFile,
      map: lowerFile,
      layer: largest.layer,
      sector: largest.sector,
      warnings: lower.warnings,
      blocked_points: blocked,
      ...(process.argv.includes("--structure")
        ? {
            ray_probes: [
              {
                name: "deck thickness",
                clear: false,
                endpoints: [
                  [0, 0, 110],
                  [0, 0, 80],
                ],
              },
              {
                name: "open underpass",
                clear: true,
                endpoints: [
                  [-60, 0, 40],
                  [60, 0, 40],
                ],
              },
              {
                name: "railing gap",
                clear: true,
                endpoints: [
                  [-40, 5, 98.5],
                  [40, 5, 98.5],
                ],
              },
              {
                name: "upper railing",
                clear: false,
                endpoints: [
                  [-40, 5, 103],
                  [40, 5, 103],
                ],
              },
              {
                name: "railing post",
                clear: false,
                endpoints: [
                  [-40, 0.2, 98.5],
                  [40, 0.2, 98.5],
                ],
              },
              {
                name: "cross brace",
                clear: false,
                endpoints: [
                  [0, 15, 51.5],
                  [0, 35, 51.5],
                ],
              },
              {
                name: "below cross brace",
                clear: true,
                endpoints: [
                  [0, 15, 10],
                  [0, 35, 10],
                ],
              },
            ].map((probe) => ({
              ...probe,
              endpoints: probe.endpoints.map((point) =>
                sceneToGame(camera, applyAffineMatrix(matrix, point)),
              ),
            })),
          }
        : {}),
      routes: [
        [
          [3.125, -130.375, 0],
          [3.125, 130.875, 0],
        ],
        headroom
          ? [
              [-160.375, 5.125, 0],
              [160.875, 5.125, 0],
            ]
          : [
              [-80.375, 5.125, 0],
              [80.875, 5.125, 0],
            ],
      ].map((pair) => pair.map(project)),
    });
  }
}
await fs.writeFile(
  `${output}/diagnostics.json`,
  JSON.stringify({ scope: "static-geometry-only-not-gameplay-parity", complete: true, results }),
);
await fs.writeFile(`${output}/mismatched-landings.json`, JSON.stringify(landingChecks, null, 2));
if (process.argv.includes("--structure") && !published) {
  const gameplay = structuredClone(descriptor.gameplay);
  gameplay.draft.issues = [
    "Fitted timber collision and 80-unit upright headroom pass native route and ray checks; textured actor compositing remains unverified.",
  ];
  await fs.writeFile(
    `${output}/gameplay-edits.json`,
    JSON.stringify([{ asset: review.asset, descriptorSha256: review.descriptorSha256, gameplay }]),
  );
}
console.log(
  JSON.stringify({
    output,
    placements: results.length,
    surfaces: surfaces.length,
    endSockets: 2,
    rejectedLandingChecks: landingChecks.length,
    supportVolumes: descriptor.gameplay.volumes?.length ?? 0,
  }),
);
