import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const [directory, stage] = process.argv.slice(2);
assert.ok(
  directory,
  "Supply an asset directory relative to library, and optionally a gameplay stage",
);
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const bytes = await fs.readFile(`library/${directory}/asset.json`);
const descriptor = JSON.parse(bytes);
const descriptorSha256 = hash(bytes);
const modelSha256 = hash(await fs.readFile(`library/${directory}/${descriptor.model}`));
if (stage) {
  const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
  const edit = edits.find((candidate) => candidate.asset === descriptor.id);
  assert.ok(edit, "Stage has no edit for this asset");
  assert.equal(edit.descriptorSha256, descriptorSha256);
  assert.equal(edit.modelSha256, modelSha256);
  descriptor.gameplay = edit.gameplay;
}
validateAssetGameplay(descriptor.gameplay, descriptor);
const doorCount = descriptor.gameplay.interiors.reduce((sum, room) => sum + room.doors.length, 0);
assert.ok(doorCount > 0, "Asset has no interior entrances");
const reference = {
  id: descriptor.id,
  descriptor: `${directory}/asset.json`,
  descriptor_sha256: descriptorSha256,
  model: `${directory}/${descriptor.model}`,
  model_sha256: modelSha256,
  model_scene: descriptor.model_scene,
  resources: descriptor.resources,
};
const output = await fs.mkdtemp("work/needs-review/map-compile/asset-building-doors-");
console.log(output);
const results = [];
const save = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      scope: "copied-asset-entrances-on-authored-flat-terrain-not-rendered-parity",
      asset: descriptor.id,
      descriptorSha256,
      modelSha256,
      stage,
      complete,
      results,
    }),
  );
await save(false);
for (const height of [0, 40])
  for (const rotation of [0, 37, 90, 180]) {
    let document = {
      version: 1,
      map: "Asset building entrances",
      size: [1600, 1200],
      camera: { kind: "oblique-orthographic", elevation_deg: 35 },
      objects: [],
      groups: [],
      sceneAssets: [],
      assetSources: [],
      terrain: createTerrainGrid([0, 0, 1600, 1200], 400, height),
    };
    for (const x of [450.25, 1150.25]) {
      const inserted = insertProjectionAsset(document, descriptor, reference, [x, 600.75, height]);
      document = inserted.document;
      document.groups.find((group) => group.id === inserted.selection.id).transform.rot_deg =
        rotation;
    }
    const compiled = compileMap(
      document,
      [0, 0, 1600, 1200],
      new Map([[descriptor.id, descriptor]]),
      { bestEffort: true },
    );
    const doors =
      compiled.descriptor.asset_geometry.buildings?.flatMap(({ Building }) => Building.doors) ?? [];
    assert.equal(doors.length, doorCount * 2, "Every copied entrance must bind to terrain");
    const file = `${rotation}-${height}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
    results.push({
      file,
      rotation,
      height,
      building_approaches: doors.map((door) => ({
        point_out: door.point_out,
        layer: door.layer_out,
        sector: door.sector_out,
        source_world: [door.point_out[0], door.point_out[1] + height, height],
      })),
      warnings: compiled.warnings,
    });
    await save(false);
  }
await save(true);
