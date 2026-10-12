import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";

const [stage, mode] = process.argv.slice(2);
assert.ok(stage && (mode === undefined || mode === "--ground"));
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
const source = await readStoredMap("library/scenes/york.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", source.assetSources, source.sceneAssets);
for (const edit of edits) {
  assert.equal(
    source.assetSources.find((source) => source.id === edit.asset)?.descriptor_sha256,
    edit.descriptorSha256,
    `Stale gameplay stage: ${edit.asset}`,
  );
  assets.get(edit.asset).gameplay = edit.gameplay;
}
const output = await fs.mkdtemp("work/needs-review/map-compile/york-component-exports-");
console.log(output);
const results = [];
const report = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      scope: "static-geometry-only-not-gameplay-parity",
      note: "Separate support layouts test low-ground masks or raised-door bindings. No combined multi-height layout, ZIP or rendered coverage.",
      complete,
      results,
    }),
  );
await report(false);
for (const edit of edits) {
  const group = source.groups.find((group) => group.id === edit.asset);
  assert.ok(group);
  const objects = source.objects.filter((object) => object.group === group.id);
  assert.ok(objects.length);
  const doorHeights = (edit.gameplay.interiors ?? []).flatMap((room) =>
    room.doors.map((door) => door.outside[2]),
  );
  assert.ok(doorHeights.every((height) => height === doorHeights[0]));
  const groundHeight =
    mode !== "--ground" && doorHeights.length ? group.transform.dz + 20 + doorHeights[0] : 20;
  for (const degrees of [0, 37, 90, 180]) {
    const document = {
      version: 1,
      map: `${edit.asset}-${degrees}`,
      size: [3000, 3000],
      camera: source.camera,
      terrain: createTerrainGrid([0, 0, 3000, 3000 + groundHeight], 4000, groundHeight),
      sceneAssets: [],
      assetSources: source.assetSources.filter((source) => source.id === edit.asset),
      objects: structuredClone(objects),
      groups: [
        {
          ...structuredClone(group),
          transform: { dx: 1100, dy: 1100, dz: group.transform.dz + 20, rot_deg: degrees },
        },
      ],
    };
    const compiled = compileMap(document, [0, 0, 3000, 3000], assets, { bestEffort: true });
    const geometry = compiled.descriptor.asset_geometry;
    const doors = (geometry.buildings ?? []).flatMap((building) => building.Building?.doors ?? []);
    if (mode !== "--ground") assert.equal(doors.length, doorHeights.length, document.map);
    if (mode === "--ground")
      assert.ok((geometry.masks?.length ?? 0) >= (edit.gameplay.masks?.length ?? 0), document.map);
    const file = `${document.map}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    results.push({
      map: document.map,
      file,
      warnings: compiled.warnings,
      building_approaches: doors.map((door) => ({
        point_out: door.point_out,
        layer: door.layer_out,
        sector: door.sector_out,
        source_world: [door.point_out[0], door.point_out[1] + groundHeight, groundHeight],
      })),
    });
  }
  await report(false);
  console.log(`${edit.asset}: compiled four placements`);
}
await report(true);
const buildings = results.filter((result) => result.building_approaches.length);
if (buildings.length) {
  await fs.mkdir(`${output}/building-round-trips`);
  await fs.writeFile(
    `${output}/building-round-trips/diagnostics.json`,
    JSON.stringify({
      complete: true,
      results: buildings.map((result) => ({ ...result, file: `../${result.file}` })),
    }),
  );
}
