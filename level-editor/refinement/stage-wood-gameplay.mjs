// Stage physical wood collision from a hash-pinned asset mesh audit.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { compileMap } from "../app/src/map-compile.ts";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh } from "../pipeline/src/mask-recovery-mesh.ts";
import { sceneToGame, gltfToScene } from "../shared/src/geometry.ts";

const [audit] = process.argv.slice(2);
assert.ok(audit, "Supply a physical-mesh audit directory produced with --caps");
const id =
  process.argv.find((arg) => arg.startsWith("--asset="))?.slice(8) ??
  "croisement03-stream-fallen-log";
const report = JSON.parse(await fs.readFile(`${audit}/report.json`));
const reviewed = report.results.find((result) => result.id === id);
const crownNode = process.argv.find((arg) => arg.startsWith("--crown-node="))?.slice(13);
const groundOption = process.argv.find((arg) => arg.startsWith("--ground="));
const crownGround = groundOption ? Number(groundOption.slice(9)) : undefined;
const rootEmbedOption = process.argv.find((arg) => arg.startsWith("--root-embed="));
const rootEmbed = rootEmbedOption ? Number(rootEmbedOption.slice(13)) : 0;
assert.ok(Number.isFinite(rootEmbed) && rootEmbed >= 0, "Invalid root embedding allowance");
assert.ok(!rootEmbedOption || crownNode, "Root embedding requires crown ownership");
assert.ok(
  !crownNode || Number.isFinite(crownGround),
  "A crown needs a reviewed local --ground height",
);
assert.ok(reviewed);
const physicalParts = reviewed.parts.filter((part) => part.cappedVolumes > 0);
assert.equal(physicalParts.length, 1, "Review exactly one physical wood part");
const physical = physicalParts[0];
assert.ok(
  reviewed.parts.every((part) => part === physical || part.node === crownNode),
  "Every nonphysical part needs explicit crown ownership",
);
assert.equal(reviewed.parts.length, crownNode ? 2 : 1);
const entry = JSON.parse(await fs.readFile("library/3d-assets/index.json")).assets.find(
  (entry) => entry.id === id,
);
assert.ok(entry);
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
assert.equal(hash(bytes), reviewed.descriptor_sha256);
assert.equal(hash(bytes), entry.descriptor_sha256);
const descriptor = JSON.parse(bytes);
assert.equal(descriptor.gameplay, undefined);
const reference = {
  id,
  role: "objects",
  model: `3d-assets/${entry.model}`,
  model_sha256: hash(await fs.readFile(`library/3d-assets/${entry.model}`)),
  descriptor: `3d-assets/${entry.descriptor}`,
  descriptor_sha256: hash(bytes),
  model_scene: entry.model_scene,
  resources: descriptor.resources ?? [],
};
assert.equal(reference.model_sha256, reviewed.model_sha256);
const caps = JSON.parse(
  await fs.readFile(`${audit}/${id}-${reviewed.parts.indexOf(physical)}-caps.json`),
);
assert.equal(caps.length, physical.cappedVolumes);
const discarded = [];
const retained = caps.filter((points, id) => {
  const a = points[0];
  const determinants = points.slice(1, -1).map((b, i) => {
    const c = points[i + 2];
    return (b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y);
  });
  if (Math.max(...determinants.map(Math.abs)) >= 1e-7) return true;
  discarded.push({ id, area: Math.abs(determinants.reduce((sum, value) => sum + value, 0)) / 2 });
  return false;
});
const discardedArea = discarded.reduce((sum, item) => sum + item.area, 0);
assert.ok(discardedArea < 1e-5, "Discarded numerical slivers exceed the authoring area budget");
const node = physical.node;
assert.ok(descriptor.parts.some((part) => part.node === node));
descriptor.gameplay = {
  version: 1,
  collision: "none",
  surfaces: [],
  doors: [],
  volumes: retained.map((points, i) => ({
    id: `wood-${i}`,
    node,
    movementHeadroom: 80,
    shape: {
      points,
      solid: true,
      opaque: true,
      mouse: true,
      show_shadow_polygon: true,
      default_material: 1,
    },
  })),
  draft: {
    issues: [
      "Mesh-derived wood collision is under review. Native movement, sight/projectile contact and rendered integration are not yet certified; no traversal surface or jump is authored.",
      `Removed ${discarded.length} numerically degenerate cap fragments with total footprint area ${discardedArea} square game units.`,
      `Dense mesh-derived collision uses ${retained.length} capped pieces. Integer-grid movement fragmentation and runtime cost need further review.`,
      ...(physical.simplifications?.length
        ? [
            `Physical mesh was simplified independently per shell with maximum reported approximate appearance error ${Math.max(...physical.simplifications.map((item) => item.approximateError))}; this is not a certified contact displacement bound.`,
          ]
        : []),
      ...(physical.decimations?.length
        ? physical.decimations.map(
            (item) =>
              `Physical shell ${item.component} was decimated with ${item.method}; sampled deviations are ${item.sourceToCandidate.maximumDistance} source-to-candidate and ${item.candidateToSource.maximumDistance} candidate-to-source game units. These samples are not a certified contact displacement bound.`,
          )
        : []),
    ],
  },
};
if (crownNode) {
  assert.ok(descriptor.parts.some((part) => part.node === crownNode));
  const model = await loadSceneModel("library", reference);
  assert.ok(
    model
      .getRoot()
      .listMaterials()
      .every((material) => material.getAlphaMode() === "OPAQUE" && material.getDoubleSided()),
    "This crown authoring path requires opaque, double-sided materials",
  );
  const triangles = maskRecoveryMesh(model, crownNode, (p) =>
    sceneToGame({ kind: "oblique-orthographic", elevation_deg: 35 }, gltfToScene(p)),
  );
  assert.ok(triangles.length);
  const points = [
    ...new Map(triangles.flat().map(([x, y]) => [JSON.stringify([x, y]), [x, y]])).values(),
  ].sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  const cross = (a, b, c) => (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
  const chain = (points) => {
    const result = [];
    for (const point of points) {
      while (result.length > 1 && cross(result.at(-2), result.at(-1), point) <= 0) result.pop();
      result.push(point);
    }
    return result.slice(0, -1);
  };
  const boundary = [...chain(points), ...chain(points.toReversed())].map(([x, y]) => [
    x,
    y,
    crownGround,
  ]);
  assert.ok(boundary.length >= 3);
  descriptor.gameplay.placementGroundHeight = crownGround;
  descriptor.gameplay.maskOcclusionNodes = [crownNode];
  descriptor.gameplay.masks = [
    {
      id: "crown-cover",
      node: crownNode,
      triangles,
      cullBackfaces: false,
      anchor: [0, 0, crownGround],
      ...(rootEmbed
        ? {
            receiverPolylines: [[0, 0, crownGround], ...boundary].map(([x, y, z]) => [
              [x, y, z],
              [x, y, z + rootEmbed],
            ]),
          }
        : { receiverPoints: [[0, 0, crownGround], ...boundary] }),
      view: true,
      characterBoundary: boundary,
      projectileBoundary: boundary,
      obstacles: [],
    },
  ];
  descriptor.gameplay.draft.issues.push(
    "Open crown geometry supplies opaque, double-sided occlusion only, with no invented solid volume. Its ground-plane canopy envelope is an authored approximation; rendered masking and sloped receiving terrain need review.",
  );
  if (rootEmbed)
    descriptor.gameplay.draft.issues.push(
      `Crown receiving probes allow the rooted asset to embed up to ${rootEmbed} game units into terrain; they do not reach ground below the asset's reviewed base.`,
    );
}
validateAssetGameplay(descriptor.gameplay, descriptor);
const output = await fs.mkdtemp("work/map-compile/wood-gameplay-");
console.log(JSON.stringify({ output, caps: retained.length, discardedArea }));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({ audit, discarded, discardedArea, retained: retained.length }),
);
const results = [];
const save = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      scope: "static-geometry-only-not-gameplay-parity",
      complete,
      results,
    }),
  );
await save(false);
await fs.writeFile(
  `${output}/edits.json`,
  JSON.stringify([
    {
      asset: id,
      descriptorSha256: hash(bytes),
      modelSha256: reference.model_sha256,
      gameplay: descriptor.gameplay,
    },
  ]),
);
for (const elevation of [0, 40])
  for (const rotation of [0, 37, 90, 180, 270]) {
    const empty = {
      version: 1,
      map: id,
      camera: { kind: "oblique-orthographic", elevation_deg: 35 },
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
    try {
      const compiled = compileMap(document, [0, 0, 1000, 1000], new Map([[id, descriptor]]), {
        bestEffort: false,
      });
      const file = `${id}-${elevation}-${rotation}.level.json`;
      await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
      await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
      const geometry = compiled.descriptor.asset_geometry;
      if (crownNode) {
        assert.equal(geometry.masks.length, 1, "Crown must bind one terrain mask");
        assert.ok(geometry.masks[0].mask_data.length, "Crown needs encoded pixel coverage");
        if (elevation === 0 && rotation === 0) {
          const floating = structuredClone(document);
          floating.terrain = createTerrainGrid([0, 0, 1000, 1000], 250, elevation - 1);
          assert.throws(
            () =>
              compileMap(floating, [0, 0, 1000, 1000], new Map([[id, descriptor]]), {
                bestEffort: false,
              }),
            /receiving anchor/,
          );
          if (rootEmbed) {
            const buried = structuredClone(document);
            buried.groups[0].transform.dz -= rootEmbed / 2;
            assert.equal(
              compileMap(buried, [0, 0, 1000, 1000], new Map([[id, descriptor]]), {
                bestEffort: false,
              }).descriptor.asset_geometry.masks.length,
              1,
            );
            buried.groups[0].transform.dz -= rootEmbed;
            assert.throws(
              () =>
                compileMap(buried, [0, 0, 1000, 1000], new Map([[id, descriptor]]), {
                  bestEffort: false,
                }),
              /receiving anchor/,
            );
          }
        }
      }
      const wood = geometry.sight_obstacles.filter((shape) => shape.projection_area === null);
      assert.equal(wood.length, retained.length);
      const footprintArea = (shape) =>
        Math.abs(
          shape.points.reduce((sum, p, i) => {
            const q = shape.points[(i + 1) % shape.points.length];
            return sum + p.x * q.y - q.x * p.y;
          }, 0),
        ) / 2;
      const selected = [...wood].sort((a, b) => footprintArea(b) - footprintArea(a)).slice(0, 12);
      const all = wood.flatMap((shape) => shape.points);
      const minX = Math.min(...all.map((p) => p.x)),
        maxX = Math.max(...all.map((p) => p.x));
      const minY = Math.min(...all.map((p) => p.y));
      const highest = Math.max(...all.map((p) => p.z_top));
      const ray_probes = selected.map((shape, index) => {
        const mean = (key) =>
          shape.points.reduce((sum, p) => sum + p[key], 0) / shape.points.length;
        const x = mean("x"),
          y = mean("y"),
          middle = (mean("z_bottom") + mean("z_top")) / 2;
        return {
          name: `wood-${index}-contact`,
          clear: false,
          endpoints: [
            [x, y, middle],
            [x, y, highest + 5],
          ],
        };
      });
      ray_probes.push({
        name: "above-wood",
        clear: true,
        endpoints: [
          [minX - 20, minY, highest + 5],
          [maxX + 20, minY, highest + 5],
        ],
      });
      const ground = geometry.sight_obstacles.find((shape) => shape.projection_area !== null)
        .points[0].z_top;
      const shapes = wood.map((shape) => ({
        points: shape.points,
        bottom: heightPlane(shape.points.map((p) => [p.x, p.y, p.z_bottom])),
        top: heightPlane(shape.points.map((p) => [p.x, p.y, p.z_top])),
      }));
      let gapCount = 0;
      for (const candidate of [...wood].sort((a, b) => footprintArea(b) - footprintArea(a))) {
        const p = candidate.points.reduce(
          (sum, p) => [
            sum[0] + p.x / candidate.points.length,
            sum[1] + p.y / candidate.points.length,
          ],
          [0, 0],
        );
        const intervals = shapes
          .filter((shape) =>
            shape.points.every((a, i) => {
              const b = shape.points[(i + 1) % shape.points.length];
              return (b.x - a.x) * (p[1] - a.y) - (b.y - a.y) * (p[0] - a.x) >= -1e-8;
            }),
          )
          .map((shape) => [planeHeight(shape.bottom, p), planeHeight(shape.top, p)])
          .sort((a, b) => a[0] - b[0]);
        let end = ground;
        for (const [bottom, top] of intervals) {
          if (bottom - end > 1 && end >= ground) {
            ray_probes.push({
              name: `wood-gap-${gapCount++}`,
              clear: true,
              endpoints: [
                [...p, end + (bottom - end) / 3],
                [...p, bottom - (bottom - end) / 3],
              ],
            });
          }
          end = Math.max(end, top);
          if (gapCount >= 12) break;
        }
        if (gapCount >= 12) break;
      }
      if (id.includes("fence")) assert.ok(gapCount > 0, "Fence review needs rail-gap probes");
      const layer = geometry.motion_data.layers.findIndex((layer) => layer.length > 0);
      const groundContact = [...wood]
        .sort((a, b) => footprintArea(b) - footprintArea(a))
        .find((shape) => {
          const bottom = shape.points.reduce((sum, p) => sum + p.z_bottom / shape.points.length, 0);
          const top = shape.points.reduce((sum, p) => sum + p.z_top / shape.points.length, 0);
          return bottom < ground + 80 && top > ground;
        });
      assert.ok(groundContact, "Wood fixture requires collision at walking height");
      const centre = groundContact.points.reduce(
        (sum, p) => [
          sum[0] + p.x / groundContact.points.length,
          sum[1] + p.y / groundContact.points.length,
        ],
        [0, 0],
      );
      const movement_probes = [
        {
          start: [minX - 20, centre[1] - ground],
          end: [centre[0], centre[1] - ground],
          layer,
          reachable: false,
        },
        {
          start: [minX - 20, minY - ground - 20],
          end: [maxX + 20, minY - ground - 20],
          layer,
          reachable: true,
        },
      ];
      results.push({
        file,
        map: file,
        rotation,
        elevation,
        warnings: compiled.warnings,
        sightObstacles: compiled.descriptor.asset_geometry.sight_obstacles.length,
        ray_probes,
        movement_probes,
      });
    } catch (error) {
      results.push({ rotation, elevation, error: String(error), stack: error.stack });
    }
    const result = results.at(-1);
    console.log(
      JSON.stringify({
        rotation,
        elevation,
        file: result.file,
        error: result.error,
        sightObstacles: result.sightObstacles,
        warningCount: result.warnings?.length,
      }),
    );
    await save(false);
  }
const complete = results.every((result) => !result.error);
await save(complete);
if (!complete) process.exitCode = 1;
