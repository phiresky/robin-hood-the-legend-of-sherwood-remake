import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";

const [stage, placements] = process.argv.slice(2);
assert.ok(stage && placements, "Provide ownership edits and compiled placement directory");
const output = await fs.mkdtemp("work/map-compile/nottingham-road-ownership-");
const results = [];
const placed = JSON.parse(await fs.readFile(`${placements}/diagnostics.json`, "utf8"));
assert.equal(placed.complete, true);
for (const { file } of placed.results) {
  const bytes = await fs.readFile(`${placements}/${file}`);
  const descriptor = JSON.parse(bytes);
  const lift = descriptor.asset_geometry.lifts[0];
  assert.ok(lift.physical_navigation);
  const low = lift.physical_navigation.doors[0].outside;
  const high = lift.physical_navigation.doors[1].outside;
  await fs.writeFile(`${output}/${file}`, bytes);
  results.push({
    file,
    layer: 0,
    sector: 0,
    same_receiver_routes: true,
    routes: [
      [
        [100, 200],
        [150, 200],
      ],
    ],
    blocked_points: [[high[0], high[1] - low[2]]],
  });
}
const document = await readStoredMap("library/scenes/nottingham.rhlos-map.json", "library");
const assets = await pinnedDescriptors(
  "library",
  document.assetSources ?? [],
  document.sceneAssets,
);
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
const asset = edits.find((edit) => edit.asset !== "nottingham-terrain")?.asset;
const restoredRoutes = {
  "nottingham-southeast-road-props": [
    [1666, 1675],
    [1700, 1670],
  ],
  "nottingham-southwest-prison-road-props": [
    [640, 2320],
    [680, 2305],
  ],
};
const restoredRoute = restoredRoutes[asset];
assert.ok(restoredRoute, "Unknown road platform ownership fixture");
for (const edit of edits) assets.get(edit.asset).gameplay = edit.gameplay;
assert.ok(document.groups.some((g) => g.id === asset));
document.objects = document.objects.filter((o) => o.group !== asset);
document.groups = document.groups.filter((g) => g.id !== asset);
const compiled = compileMap(document, document.exportBounds ?? [0, 0, ...document.size], assets, {
  bestEffort: true,
});
const file = "nottingham-without-road-platform.level.json";
await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
const layers = compiled.descriptor.asset_geometry.motion_data.layers;
const areas = layers[16]
  .map((area, index) => ({ area, index }))
  .filter(({ area }) =>
    restoredRoute.every((point) => pointInGameplayPolygon(point, area.polygon.points, true)),
  );
assert.equal(areas.length, 1, "Restored route needs one receiving motion region");
results.push({
  file,
  layer: 16,
  sector: layers.slice(0, 16).reduce((n, l) => n + l.length, 0) + areas[0].index,
  same_receiver_routes: true,
  routes: [restoredRoute],
});
await fs.writeFile(
  `${output}/diagnostics.json`,
  JSON.stringify({
    scope: "asset-owned-low-deck-collision-and-restored-terrain",
    complete: true,
    results,
  }),
);
console.log(
  JSON.stringify({
    output,
    placements: placed.results.length,
    blockedPoints: placed.results.length,
    restoredTerrainRoutes: 2,
  }),
);
