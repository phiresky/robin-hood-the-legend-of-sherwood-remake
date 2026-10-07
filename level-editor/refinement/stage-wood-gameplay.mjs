// Stage physical wood collision from a hash-pinned asset mesh audit.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { compileMap } from "../app/src/map-compile.ts";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";

const [audit] = process.argv.slice(2);
assert.ok(audit, "Supply a physical-mesh audit directory produced with --caps");
const id =
  process.argv.find((arg) => arg.startsWith("--asset="))?.slice(8) ??
  "croisement03-stream-fallen-log";
const report = JSON.parse(await fs.readFile(`${audit}/report.json`));
const reviewed = report.results.find((result) => result.id === id);
assert.ok(reviewed && reviewed.parts.length === 1 && reviewed.parts[0].cappedVolumes > 0);
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
const caps = JSON.parse(await fs.readFile(`${audit}/${id}-0-caps.json`));
assert.equal(caps.length, reviewed.parts[0].cappedVolumes);
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
const node = descriptor.parts[0].node;
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
      ...(reviewed.parts[0].simplifications?.length
        ? [
            `Physical mesh was simplified independently per shell with maximum reported approximate appearance error ${Math.max(...reviewed.parts[0].simplifications.map((item) => item.approximateError))}; this is not a certified contact displacement bound.`,
          ]
        : []),
      ...(reviewed.parts[0].decimations?.length
        ? reviewed.parts[0].decimations.map(
            (item) =>
              `Physical shell ${item.component} was decimated with ${item.method}; sampled deviations are ${item.sourceToCandidate.maximumDistance} source-to-candidate and ${item.candidateToSource.maximumDistance} candidate-to-source game units. These samples are not a certified contact displacement bound.`,
          )
        : []),
    ],
  },
};
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
      const centre = selected[0].points.reduce(
        (sum, p) => [
          sum[0] + p.x / selected[0].points.length,
          sum[1] + p.y / selected[0].points.length,
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
