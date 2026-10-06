import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { groupCentroid } from "../shared/src/level3d.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";

const [stage, mode] = process.argv.slice(2);
assert.ok(stage && (mode === undefined || mode === "--published" || mode === "--physical-terrace"));
const physicalTerrace = mode === "--physical-terrace";
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
const stair = edits[0].asset;
const map = stair.startsWith("york-") ? "york" : "nottingham";
const source = await readStoredMap(`library/scenes/${map}.rhlos-map.json`, "library");
const assets = await pinnedDescriptors("library", source.assetSources, source.sceneAssets);
const neighbours = {
  "york-east-riverside-southern-wall-stair": physicalTerrace
    ? ["york-southeast-riverside-raised-terrace"]
    : [],
  "york-east-riverside-curtain-wall": ["york-east-water-gate-south-bastion"],
  "york-castle-courtyard-lodge-stairs": [
    "york-castle-courtyard-raised-terrain",
    "york-castle-courtyard-rear-curtain-wall",
  ],
  "nottingham-north-wall-stair": ["nottingham-north-curtain-wall"],
  "nottingham-southwest-wall-stair": ["nottingham-southwest-curtain-wall-north"],
  "nottingham-castle-west-stair": [
    "nottingham-castle-courtyard-ground",
    "nottingham-castle-west-courtyard-wall",
  ],
  "nottingham-castle-upper-stair": [
    "nottingham-castle-courtyard-ground",
    "nottingham-castle-upper-wall",
    "nottingham-castle-east-courtyard-wall",
  ],
};
assert.ok(neighbours[stair], "Unknown reviewed stair assembly");
const ids = [stair, ...neighbours[stair]];
for (const id of ids) {
  const edit = edits.find((e) => e.asset === id);
  assert.ok(assets.has(id));
  if (!edit) continue;
  if (mode === "--published") assert.deepEqual(assets.get(id).gameplay, edit.gameplay);
  else assets.get(id).gameplay = edit.gameplay;
}
const liftCount = assets.get(stair).gameplay.lifts.length;
const northWall = stair === "nottingham-north-wall-stair";
const southernRiverside = stair === "york-east-riverside-southern-wall-stair";
assert.ok(!physicalTerrace || southernRiverside);
const riverside = stair === "york-east-riverside-curtain-wall" || southernRiverside;
const terrainHeight = southernRiverside ? 50.001003 : riverside ? 90.00101 : 0;
const size = northWall || riverside ? [7000, 6500] : [5000, 4500];
const centers =
  northWall || riverside
    ? [
        [2200, 2500],
        [4500, 3500],
      ]
    : [
        [1500, 1700],
        [3200, 2600],
      ];
const output = await fs.mkdtemp(`work/map-compile/${stair}-neighbour-placements-`);
const results = [],
  rejected = [];
for (const height of [0, 40])
  for (const rotation of [0, 37, 90, 180]) {
    const document = {
      version: 1,
      map: "courtyard-west-stair-neighbours",
      camera: source.camera,
      size,
      objects: [],
      groups: [],
      sceneAssets: source.sceneAssets.filter((asset) => ids.includes(asset.id)),
      assetSources: source.assetSources.filter((asset) => ids.includes(asset.id)),
      ...(["nottingham-southwest-wall-stair", "nottingham-north-wall-stair"].includes(stair) ||
      (riverside && !physicalTerrace)
        ? { terrain: createTerrainGrid([0, 0, ...size], 1000, height + terrainHeight) }
        : {}),
    };
    const radians = (rotation * Math.PI) / 180,
      sinT = Math.sin((source.camera.elevation_deg * Math.PI) / 180);
    const rotate = ([x, y]) => [
      x * Math.cos(radians) - (y * Math.sin(radians)) / sinT,
      x * Math.sin(radians) * sinT + y * Math.cos(radians),
    ];
    for (const [copy, center] of centers.entries())
      for (const id of ids) {
        const group = structuredClone(source.groups.find((g) => g.id === id));
        assert.ok(group && group.transform.rot_deg === 0);
        const objects = source.objects.filter((o) => o.group === id);
        const pivot = groupCentroid(objects),
          rotatedPivot = rotate(pivot);
        const origin = rotate([group.transform.dx - 1500, group.transform.dy - 1500]);
        group.id = `copy${copy}/${id}`;
        group.transform = {
          dx: center[0] + origin[0] - pivot[0] + rotatedPivot[0],
          dy: center[1] + origin[1] - pivot[1] + rotatedPivot[1],
          dz: group.transform.dz + height,
          rot_deg: rotation,
        };
        document.groups.push(group);
        document.objects.push(
          ...objects.map((o) => ({
            ...structuredClone(o),
            id: `copy${copy}/${o.id}`,
            group: group.id,
          })),
        );
      }
    const compile = (d) => compileMap(d, [0, 0, ...d.size], assets, { bestEffort: true });
    const compiled = compile(document);
    assert.equal(
      compiled.descriptor.asset_geometry.lifts?.length,
      2 * liftCount,
      compiled.warnings.join("\n"),
    );
    assert.ok(compiled.descriptor.asset_geometry.lifts.every((l) => l.physical_navigation));
    const file = `courtyard-west-stair-${height}-${rotation}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
    results.push({ file, map: file, warnings: compiled.warnings });
    if (riverside && !physicalTerrace)
      for (const kind of ["missing", "raised"]) {
        const changed = structuredClone(document);
        if (kind === "missing") delete changed.terrain;
        else
          changed.terrain = createTerrainGrid([0, 0, ...size], 1000, height + terrainHeight + 20);
        const invalid = compile(changed);
        assert.equal(
          invalid.descriptor.asset_geometry.lifts?.length ?? 0,
          0,
          `${file}: ${kind} terrain retained an unsupported stair`,
        );
        rejected.push({ file, group: "$terrain", kind, warnings: invalid.warnings });
      }
    for (const group of document.groups.filter((g) => !g.id.endsWith(`/${stair}`)))
      for (const kind of ["missing", "raised"]) {
        const changed = structuredClone(document);
        if (kind === "missing") {
          changed.groups = changed.groups.filter((g) => g.id !== group.id);
          changed.objects = changed.objects.filter((o) => o.group !== group.id);
        } else changed.groups.find((g) => g.id === group.id).transform.dz += 20;
        const invalid = compile(changed);
        const remaining = invalid.descriptor.asset_geometry.lifts?.length ?? 0;
        assert.ok(
          remaining >= liftCount && remaining < 2 * liftCount,
          `${file}: ${group.id} ${kind} retained ${remaining} lifts`,
        );
        rejected.push({ file, group: group.id, kind, warnings: invalid.warnings });
      }
  }
await fs.writeFile(
  `${output}/diagnostics.json`,
  JSON.stringify({
    scope: "static-geometry-only-not-gameplay-parity",
    complete: true,
    gameplayStage: stage,
    results,
  }),
);
await fs.writeFile(`${output}/rejected.json`, JSON.stringify(rejected));
console.log(
  JSON.stringify({
    output,
    exports: results.length,
    copies: results.length * 2,
    rejected: rejected.length,
  }),
);
