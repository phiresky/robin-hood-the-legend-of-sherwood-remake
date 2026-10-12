// Inspect placed asset-floor support without compiling or reading source levels.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { gameToScene } from "../shared/src/scene.ts";
import { applyAffineMatrix, sceneToGame } from "../shared/src/geometry.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";
import { terrainHeightAt } from "../shared/src/authored-terrain.ts";

const [map, assetId] = process.argv.slice(2);
assert.ok(map && assetId, "Supply scene name and lift asset ID");
const document = await readStoredMap(`library/scenes/${map}.rhlos-map.json`, "library");
const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
const instances = [];
for (const [id, asset] of assets) {
  const parts = document.objects.filter((part) => part.node.startsWith(`asset:${id}:`));
  for (const group of new Set(parts.map((part) => part.group))) {
    const frames = new Map(
      parts
        .filter((part) => part.group === group)
        .map((part) => [
          part.node.slice(`asset:${id}:`.length),
          partMatrix(document.camera, document, part),
        ]),
    );
    const place = (node, point) => {
      const matrix = frames.get(node);
      assert.ok(matrix, `Missing placed node ${id}/${node}`);
      return sceneToGame(
        document.camera,
        applyAffineMatrix(matrix, gameToScene(document.camera, ...point)),
      );
    };
    instances.push({ id, group, asset, place, frames });
  }
}
const unplacedSurfaces = [];
const surfaces = instances.flatMap(({ id, group, asset, place, frames }) =>
  (asset.gameplay?.surfaces ?? [])
    .filter((surface) => {
      if (frames.has(surface.node)) return true;
      unplacedSurfaces.push({ id, group, surface: surface.id, node: surface.node });
      return false;
    })
    .map((surface) => {
      const project = (polygon) =>
        polygon.map(([x, y], i) => {
          const z = Array.isArray(surface.height) ? surface.height[i] : surface.height;
          const p = place(surface.node, [x, y, z]);
          return [p[0], p[1] - p[2], p[2]];
        });
      const points = project(surface.polygon);
      return { id, group, surface: surface.id, points, plane: heightPlane(points) };
    }),
);
const results = [];
for (const { id, group, asset, place } of instances.filter((entry) => entry.id === assetId))
  for (const lift of asset.gameplay.lifts ?? [])
    for (const door of lift.doors) {
      const world = place(door.node, door.outside);
      const projected = [world[0], world[1] - world[2]];
      const floors = surfaces
        .filter((surface) => pointInGameplayPolygon(projected, surface.points, true))
        .map((surface) => ({
          asset: surface.id,
          group: surface.group,
          surface: surface.surface,
          height: planeHeight(surface.plane, projected),
          delta: world[2] - planeHeight(surface.plane, projected),
        }))
        .sort((a, b) => Math.abs(a.delta) - Math.abs(b.delta));
      results.push({
        asset: id,
        group,
        lift: lift.id,
        door: door.id,
        local: door.outside,
        world,
        projected,
        terrainHeight: terrainHeightAt(document, world[0], world[1]),
        floors,
      });
    }
assert.ok(results.length, "No matching placed lifts");
const output = await fs.mkdtemp("work/needs-review/map-compile/lift-support-audit-");
await fs.writeFile(
  `${output}/report.json`,
  JSON.stringify(
    {
      map,
      assetId,
      scope:
        "authored surface candidates only; excludes collisions, holes and compiled spline walls",
      unplacedSurfaces,
      results,
    },
    null,
    2,
  ),
);
console.log(output);
for (const result of results) console.log(JSON.stringify(result));
