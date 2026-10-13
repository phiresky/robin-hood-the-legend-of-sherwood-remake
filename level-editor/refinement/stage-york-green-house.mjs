// Reassemble local house metadata without coupling its awning to market events.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";

const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const read = async (directory, expected) => {
  const bytes = await fs.readFile(`library/3d-assets/${directory}/asset.json`);
  assert.equal(hash(bytes), expected);
  return JSON.parse(bytes);
};
const id = "york-central-south-green-timber-house";
const descriptorSha256 = "10a9b7a84cdb9294e696dc332da9bd95577b00f81d2407ee75886dea004ee12d";
const descriptor = await read(`york/${id}`, descriptorSha256);
const permanent = await read(
  `${id}-remainder`,
  "f2b8d85bb3d157f5eaf8d4a25a1f920991f4aa870eece345dccb0871fed8c03b",
);
const owner = await read(
  "york-market-state-assembly",
  "ad2c6e9b5b6d61858dcd551169aeff2a9781da415cfc0c11db323ea688656555",
);
assert.equal(descriptor.gameplay, undefined);
for (const source of [permanent, owner])
  assert.deepEqual(source.source_origin_scene, descriptor.source_origin_scene);
const permanentNodes = new Set(permanent.parts.map((part) => part.node));
for (const part of descriptor.parts) {
  const source = (permanentNodes.has(part.node) ? permanent : owner).parts.find(
    (p) => p.node === part.node,
  );
  assert.deepEqual(part.obstacle_local_game, source.obstacle_local_game);
}
const changing = descriptor.parts.filter((part) => !permanentNodes.has(part.node));
assert.deepEqual(
  changing.map((part) => part.node),
  ["building-968"],
);
const sourceControl = owner.gameplay.movementTransitions.find(
  (row) => row.id === "movement-change-9",
);
assert.ok(sourceControl.initialSight.includes("building-968"));
assert.ok(!sourceControl.appliedSight.includes("building-968"));
const gameplay = structuredClone(permanent.gameplay);
const door = gameplay.interiors[0].doors[0];
const ground = door.outside[2];
assert.equal(gameplay.interiors.length, 1);
assert.equal(gameplay.interiors[0].doors.length, 1);
assert.ok(Math.abs(door.inside[2] - ground) < 1e-8);
gameplay.placementGroundHeight = ground;
changing[0].appearance = { hide: ["remove-awning"] };
gameplay.movementTransitions = [
  {
    id: "remove-awning",
    node: changing[0].node,
    appearances: ["remove-awning"],
    active: sourceControl.active,
    definitive: sourceControl.definitive,
    waypoint: door.outside,
    applyPolygon: [],
    noApplyPolygon: [],
    initial: [],
    applied: [],
    initialSight: [changing[0].node],
    appliedSight: [],
  },
];
gameplay.draft.issues.push(
  "Standalone green-house draft: placement aligns the door floor; its deep foundation extends below terrain.",
  "Awning removal affects only the awning. The permanent counter, house collision and room remain; shared market triggers are not inherited.",
  "Grounded placement, entrance clearance, awning headroom and rendered contact require moved-asset review.",
);
descriptor.gameplay = gameplay;
validateAssetGameplay(gameplay, descriptor);
const base = `3d-assets/york/${id}`;
const modelSha256 = hash(await fs.readFile(`library/${base}/${descriptor.model}`));
const reference = {
  id,
  descriptor: `${base}/asset.json`,
  descriptor_sha256: descriptorSha256,
  model: `${base}/${descriptor.model}`,
  model_sha256: modelSha256,
  model_scene: descriptor.model_scene,
  resources: descriptor.resources,
};
const output = await fs.mkdtemp("work/needs-review/map-compile/york-green-house-");
console.log(output);
await fs.writeFile(
  `${output}/edits.json`,
  JSON.stringify([
    {
      asset: id,
      descriptorSha256,
      modelSha256,
      gameplay,
      partAppearances: { "building-968": changing[0].appearance },
    },
  ]),
);
const results = [];
const entranceStates = [];
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
for (const height of [0, 40])
  for (const rotation of [0, 37, 90, 180]) {
    let document = {
      version: 1,
      map: "Green house",
      size: [1000, 1000],
      camera: { kind: "oblique-orthographic", elevation_deg: 35 },
      objects: [],
      groups: [],
      sceneAssets: [],
      assetSources: [],
      terrain: createTerrainGrid([0, 0, 1000, 1000], 500, height),
    };
    for (const x of [250.25, 700.25]) {
      const inserted = insertProjectionAsset(document, descriptor, reference, [x, 500.75, height]);
      document = inserted.document;
      document.groups.find((group) => group.id === inserted.selection.id).transform.rot_deg =
        rotation;
    }
    const compiled = compileMap(document, [0, 0, 1000, 1000], new Map([[id, descriptor]]), {
      bestEffort: true,
    });
    const geometry = compiled.descriptor.asset_geometry;
    assert.equal(geometry.movement_transitions.length, 2);
    const doors = geometry.buildings.flatMap(({ Building }) => Building.doors);
    assert.equal(doors.length, 2, "Both door floors must bind to authored terrain");
    const approaches = doors.map((door) => ({
      point_out: door.point_out,
      layer: door.layer_out,
      sector: door.sector_out,
      source_world: [door.point_out[0], door.point_out[1] + height, height],
    }));
    const file = `${rotation}-${height}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
    const changingSight = new Set(
      geometry.movement_transitions.flatMap((row) => row.initial_sight),
    );
    const permanentSight = geometry.sight_obstacles
      .map((_, index) => index)
      .filter((index) => !changingSight.has(index));
    results.push({
      file,
      building_approaches: approaches,
      permanent_sight: permanentSight,
      warnings: compiled.warnings,
    });
    for (let state = 0; state < 4; state++)
      entranceStates.push({
        file: `../${file}`,
        building_approaches: approaches,
        applied_transitions: geometry.movement_transitions
          .filter((_, index) => state & (1 << index))
          .map((row) => row.id),
      });
    await save(false);
  }
await save(true);
await fs.mkdir(`${output}/entrance-states`);
await fs.writeFile(
  `${output}/entrance-states/diagnostics.json`,
  JSON.stringify({
    complete: true,
    scope: "independent-awning-states-and-room-traversal",
    results: entranceStates,
  }),
);
