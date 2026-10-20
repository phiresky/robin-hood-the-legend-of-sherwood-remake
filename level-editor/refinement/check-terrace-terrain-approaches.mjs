// Exercise an authored terrain strip meeting a placed terrace's low-edge socket.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { sceneToGame, applyAffineMatrix } from "../shared/src/geometry.ts";
import { gameToScene } from "../shared/src/scene.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";
import { orientNavigationJoin } from "../shared/src/assemble-navigation-joins.ts";

const [placements, stage, assetId, gapText = "0"] = process.argv.slice(2);
assert.ok(placements && stage && assetId);
const gap = Number(gapText);
assert.ok(Number.isFinite(gap) && gap >= 0);
const input = JSON.parse(await fs.readFile(`${placements}/diagnostics.json`));
assert.equal(input.complete, true);
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`));
const output = await fs.mkdtemp("work/needs-review/map-compile/terrace-terrain-approaches-");
console.log(output);
const results = [];
await fs.writeFile(`${output}/diagnostics.json`, JSON.stringify({ complete: false }));
for (const row of input.results) {
  const document = JSON.parse(await fs.readFile(`${placements}/${row.file}.scene.json`));
  const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
  for (const edit of edits) {
    const ref = document.assetSources.find((ref) => ref.id === edit.asset);
    if (!ref) continue;
    assert.equal(ref.descriptor_sha256, edit.descriptorSha256);
    assert.equal(ref.model_sha256, edit.modelSha256);
    assets.get(edit.asset).gameplay = edit.gameplay;
  }
  const asset = assets.get(assetId);
  assert.equal(asset.gameplay.surfaces.length, 1);
  const surface = asset.gameplay.surfaces[0];
  const local = surface.polygon.map((point, i) => [
    ...point,
    typeof surface.height === "number" ? surface.height : surface.height[i],
  ]);
  const edge = [...surface.navigationJoins].sort(
    (a, b) => a[0][2] + a[1][2] - b[0][2] - b[1][2],
  )[0];
  const socket = orientNavigationJoin(local, edge);
  const instances = document.objects.filter(
    (object) => object.node === `asset:${assetId}:${surface.node}`,
  );
  assert.equal(instances.length, row.building_approaches.length);
  const material = document.terrain.cells[0].material;
  document.terrain = { version: 1, spacing: 100, vertices: [], cells: [] };
  const sources = instances.map((object, copy) => {
    const part = asset.parts.find((part) => part.node === surface.node);
    const matrix = partMatrix(document.camera, document, {
      ...object,
      obstacle: part.obstacle_local_game,
    });
    const [a, b] = socket.map((point) =>
      sceneToGame(
        document.camera,
        applyAffineMatrix(matrix, gameToScene(document.camera, ...point)),
      ),
    );
    a[2] -= gap;
    b[2] -= gap;
    const length = Math.hypot(b[0] - a[0], b[1] - a[1]);
    assert.ok(length > 24);
    const outward = [(b[1] - a[1]) / length, (a[0] - b[0]) / length];
    const outer = (point) => [
      point[0] + outward[0] * 96,
      point[1] + outward[1] * 96,
      row.height - gap,
    ];
    const points = [a, b, outer(b), outer(a)];
    const start = document.terrain.vertices.length;
    document.terrain.vertices.push(
      ...points.map((position, i) => ({
        id: `strip-${copy}-${i}`,
        position,
        material,
      })),
    );
    document.terrain.cells.push({
      id: `strip-${copy}`,
      vertices: [0, 3, 2, 1].map((i) => start + i),
      material,
    });
    return [0, 1, 2].map((axis) => (points[0][axis] + points[1][axis] + points[2][axis]) / 3);
  });
  const compiled = compileMap(document, [0, 0, ...document.size], assets, { bestEffort: true });
  const doors = compiled.descriptor.asset_geometry.buildings.flatMap(
    ({ Building }) => Building.doors,
  );
  const approaches = row.building_approaches.map((probe, i) => {
    const matches = doors.filter((door) =>
      door.point_out.every((value, axis) => value === probe.point_out[axis]),
    );
    assert.equal(matches.length, 1);
    const source = sources[i];
    const layers = new Set(
      compiled.descriptor.asset_geometry.sight_obstacles.flatMap((shape) => {
        if (!shape.projection_area) return [];
        const vertices = shape.points.map((p) => [p.x, p.y, p.z_top]);
        if (
          !pointInGameplayPolygon(
            source.slice(0, 2),
            vertices.map(([x, y]) => [x, y]),
            true,
          )
        )
          return [];
        return Math.abs(planeHeight(heightPlane(vertices.slice(0, 3)), source) - source[2]) < 1e-4
          ? [shape.projection_area[1]]
          : [];
      }),
    );
    assert.equal(layers.size, 1, "Terrain approach must bind to one physical receiving layer");
    return {
      point_out: matches[0].point_out,
      layer: matches[0].layer_out,
      sector: matches[0].sector_out,
      source_layer: [...layers][0],
      source_world: source,
    };
  });
  await fs.writeFile(`${output}/${row.file}`, JSON.stringify(compiled.descriptor));
  await fs.writeFile(`${output}/${row.file}.scene.json`, JSON.stringify(document));
  results.push({ file: row.file, building_approaches: approaches, warnings: compiled.warnings });
}
await fs.writeFile(
  `${output}/diagnostics.json`,
  JSON.stringify({
    scope: "authored-terrain-to-terrace-doorways-awaiting-native-verification",
    complete: true,
    placements,
    stage,
    gap,
    results,
  }),
);
