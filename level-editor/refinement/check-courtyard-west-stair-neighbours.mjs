import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { groupCentroid } from "../shared/src/level3d.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";
import { assembleLiftSegments } from "../shared/src/assemble-lift-segments.ts";

const [stage, mode, selectedStair, companionStair] = process.argv.slice(2);
assert.ok(
  stage &&
    (mode === undefined ||
      mode === "--candidate" ||
      mode === "--published" ||
      mode === "--physical-terrace"),
);
const physicalTerrace = mode === "--physical-terrace";
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
const stair = selectedStair ?? edits[0].asset;
const map = stair.startsWith("york-")
  ? "york"
  : stair.startsWith("lincoln-")
    ? "lincoln"
    : "nottingham";
const source = await readStoredMap(`library/scenes/${map}.rhlos-map.json`, "library");
const assets = await pinnedDescriptors("library", source.assetSources, source.sceneAssets);
const neighbours = {
  "york-central-lane-stone-gable-house": ["york-central-lane-timber-lean-to"],
  "lincoln-keep-annex": ["lincoln-courtyard-shed"],
  "york-precinct-southwest-wall-ramp": ["york-cathedral-precinct-raised-terrain"],
  "york-east-riverside-northern-wall-stair": ["york-castle-south-curtain-wall"],
  "lincoln-inner-gatehouse": ["lincoln-castle-hill-inner-bailey-plateau"],
  "york-stone-river-bridge-and-approach-stairs": [
    "york-east-bridge-raised-terrace",
    "york-southeast-riverside-raised-terrace",
  ],
  "york-west-lane-access-steps": [
    "york-east-bridge-raised-terrace",
    ...(physicalTerrace ? ["york-southeast-riverside-raised-terrace"] : []),
  ],
  "york-precinct-east-wall-stair": ["york-cathedral-precinct-raised-terrain"],
  "york-north-garden-wall-and-stair": [],
  "york-outer-east-upper-wall-stair": ["york-outer-east-upper-curtain-wall"],
  "york-outer-southeast-wall-stair": [],
  "york-market-southwest-connecting-stairs": ["york-west-town-raised-terrain"],
  "york-riverbank-stone-landing-steps": ["york-southeast-riverside-raised-terrace"],
  "york-riverbank-wooden-landing-steps": [
    "york-southeast-riverside-raised-terrace",
    "york-east-riverside-wooden-walkway",
  ],
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
const stairs = companionStair ? [stair, companionStair] : [stair];
assert.equal(new Set(stairs).size, stairs.length);
for (const id of stairs) assert.ok(neighbours[id], "Unknown companion stair assembly");
// Collision neighbours are optional: removing one must not remove support.
const collisionNeighbours =
  stair === "york-east-riverside-northern-wall-stair"
    ? ["york-east-riverside-northern-boundary-wall"]
    : [];
const ids = [
  ...new Set([...stairs.flatMap((id) => [id, ...neighbours[id]]), ...collisionNeighbours]),
];
const originGroup = source.groups.find((group) => group.id === stair);
const westLane = stair === "york-west-lane-access-steps";
const riverBridge = stair === "york-stone-river-bridge-and-approach-stairs";
const gatehouse = stair === "lincoln-inner-gatehouse";
const keepAnnex = stair === "lincoln-keep-annex";
const sourceOrigin =
  [
    "york-north-garden-wall-and-stair",
    "york-precinct-east-wall-stair",
    "york-precinct-southwest-wall-ramp",
    "york-east-riverside-northern-wall-stair",
    "york-central-lane-stone-gable-house",
  ].includes(stair) ||
  westLane ||
  riverBridge ||
  gatehouse ||
  keepAnnex
    ? [originGroup.transform.dx, originGroup.transform.dy]
    : [1500, 1500];
for (const id of ids) {
  const edit = edits.find((e) => e.asset === id);
  assert.ok(assets.has(id));
  if (!edit) continue;
  if (mode === "--published") assert.deepEqual(assets.get(id).gameplay, edit.gameplay);
  else assets.get(id).gameplay = edit.gameplay;
}
const assemblies = stairs.map((id) => ({
  id,
  ...assembleLiftSegments(
    assets.get(id).gameplay.lifts.map((lift) => ({
      id: lift.id,
      type: lift.type,
      direction: Math.atan2(lift.direction[1], lift.direction[0]),
      joins: lift.joins ?? [],
    })),
  ),
}));
const liftCount = assemblies.reduce((count, assembly) => count + assembly.lifts.length, 0);
const bridgeNeedsTerrain =
  riverBridge &&
  assets
    .get(stair)
    .gameplay.projectionReceivers?.some((receiver) => receiver.node === "building-095");
const entranceCounts = assemblies.flatMap(({ id, identities, lifts }) =>
  lifts.map((assembledLift) =>
    assets
      .get(id)
      .gameplay.lifts.reduce(
        (count, lift) =>
          count + (identities.get(lift.id) === assembledLift.id ? lift.doors.length : 0),
        0,
      ),
  ),
);
const northWall = stair === "nottingham-north-wall-stair";
const market = stair === "york-market-southwest-connecting-stairs";
const outerWall = [
  "york-central-lane-stone-gable-house",
  "york-precinct-southwest-wall-ramp",
  "york-outer-southeast-wall-stair",
  "york-outer-east-upper-wall-stair",
  "york-north-garden-wall-and-stair",
].includes(stair);
const southernRiverside = stair === "york-east-riverside-southern-wall-stair";
const stoneRiverside = stair === "york-riverbank-stone-landing-steps";
assert.ok(!physicalTerrace || southernRiverside || westLane);
const riverside =
  stair === "york-east-riverside-curtain-wall" || southernRiverside || stoneRiverside;
const terrainHeight =
  stair === "york-central-lane-stone-gable-house"
    ? 90.00101
    : southernRiverside || westLane
      ? 50.001003
      : stoneRiverside
        ? 0
        : riverside
          ? 90.00101
          : 0;
const precinct = stair === "york-precinct-east-wall-stair";
const size = gatehouse
  ? [8000, 8000]
  : northWall || riverside || market || outerWall || precinct || westLane || riverBridge
    ? [7000, 6500]
    : [5000, 4500];
const centers = gatehouse
  ? [
      [2500, 2500],
      [5500, 5500],
    ]
  : northWall || riverside || market || outerWall || precinct || westLane || riverBridge
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
// Exercise both integer and fractional world origins: collision clipping must
// not turn a real landing gap into an apparently usable rounded connection.
const originOffsets =
  stair === "york-east-riverside-northern-wall-stair"
    ? [0, sourceOrigin[1] - Math.floor(sourceOrigin[1])]
    : [0];
for (const originOffset of originOffsets)
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
        (riverside && !physicalTerrace) ||
        market ||
        outerWall ||
        bridgeNeedsTerrain ||
        (westLane && !physicalTerrace)
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
          const origin = rotate([
            group.transform.dx - sourceOrigin[0],
            group.transform.dy - sourceOrigin[1],
          ]);
          group.id = `copy${copy}/${id}`;
          group.transform = {
            dx: center[0] + origin[0] - pivot[0] + rotatedPivot[0],
            dy: center[1] + originOffset + origin[1] - pivot[1] + rotatedPivot[1],
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
      assert.ok(
        compiled.descriptor.asset_geometry.lifts.every((l) => l.physical_navigation),
        compiled.warnings.join("\n"),
      );
      assert.deepEqual(
        compiled.descriptor.asset_geometry.lifts
          .map((lift) => lift.doors.length)
          .sort((a, b) => a - b),
        centers.flatMap(() => entranceCounts).sort((a, b) => a - b),
        "Every copied flight must retain all authored entrances",
      );
      const file = `courtyard-west-stair-${height}-${rotation}${originOffset ? "-fractional" : ""}.level.json`;
      await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
      await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
      results.push({ file, map: file, warnings: compiled.warnings });
      if (
        (riverside && !physicalTerrace) ||
        market ||
        outerWall ||
        bridgeNeedsTerrain ||
        (westLane && !physicalTerrace)
      )
        for (const kind of ["missing", "raised"]) {
          const changed = structuredClone(document);
          if (kind === "missing") delete changed.terrain;
          else
            changed.terrain = createTerrainGrid([0, 0, ...size], 1000, height + terrainHeight + 20);
          const invalid = compile(changed);
          assert.equal(
            invalid.descriptor.asset_geometry.lifts?.length ?? 0,
            // Each bridge copy retains its other flight between physical terraces.
            riverBridge ? centers.length * (liftCount - 1) : 0,
            `${file}: ${kind} terrain retained an unsupported stair`,
          );
          rejected.push({ file, group: "$terrain", kind, warnings: invalid.warnings });
        }
      for (const group of document.groups.filter(
        (g) => !stairs.some((id) => g.id.endsWith(`/${id}`)),
      ))
        for (const kind of ["missing", "raised"]) {
          const collisionOnly = collisionNeighbours.some((id) => group.id.endsWith(`/${id}`));
          if (collisionOnly && kind === "raised") continue;
          const changed = structuredClone(document);
          if (kind === "missing") {
            changed.groups = changed.groups.filter((g) => g.id !== group.id);
            changed.objects = changed.objects.filter((o) => o.group !== group.id);
          } else changed.groups.find((g) => g.id === group.id).transform.dz += 20;
          const invalid = compile(changed);
          const remaining = invalid.descriptor.asset_geometry.lifts?.length ?? 0;
          if (collisionOnly) {
            assert.equal(remaining, 2 * liftCount, `${file}: optional wall removed support`);
            continue;
          }
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
