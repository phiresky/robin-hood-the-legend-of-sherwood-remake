// Test one entrance against authored terrain at its placed height. Other doors
// remain in the asset; unsupported upper/lower entrances are reported by export.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { gameToScene } from "../shared/src/scene.ts";
import { sceneToGame, applyAffineMatrix } from "../shared/src/geometry.ts";

const [directory, stage, doorId, sourceArgument] = process.argv.slice(2);
assert.ok(directory && stage && doorId, "Supply asset directory, gameplay stage and entrance ID");
// Narrow rooftops may not support the default straight approach. A separately
// authored local standing point tests entry from that floor, not terrain contact.
const sourceLocal = sourceArgument === undefined ? undefined : JSON.parse(sourceArgument);
if (sourceLocal !== undefined)
  assert.ok(
    Array.isArray(sourceLocal) && sourceLocal.length === 2 && sourceLocal.every(Number.isFinite),
  );
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const bytes = await fs.readFile(`library/${directory}/asset.json`);
const asset = JSON.parse(bytes);
const edit = JSON.parse(await fs.readFile(`${stage}/edits.json`)).find(
  (entry) => entry.asset === asset.id,
);
assert.ok(edit);
assert.equal(hash(bytes), edit.descriptorSha256);
assert.equal(hash(await fs.readFile(`library/${directory}/${asset.model}`)), edit.modelSha256);
asset.gameplay = edit.gameplay;
const definitions = asset.gameplay.interiors
  .flatMap((room) => room.doors)
  .filter((door) => door.id === doorId);
assert.equal(definitions.length, 1);
const definition = definitions[0];
const reference = {
  id: asset.id,
  descriptor: `${directory}/asset.json`,
  descriptor_sha256: edit.descriptorSha256,
  model: `${directory}/${asset.model}`,
  model_sha256: edit.modelSha256,
  model_scene: asset.model_scene,
  resources: asset.resources,
};
const output = await fs.mkdtemp("work/needs-review/map-compile/building-entrance-on-terrain-");
console.log(output);
const results = [];
const save = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      scope: sourceLocal
        ? "selected-entrance-from-asset-local-standing-point-awaiting-native-verification"
        : "selected-entrance-on-matching-authored-terrain-awaiting-native-verification",
      sourceLocal,
      complete,
      asset: asset.id,
      stage,
      doorId,
      results,
    }),
  );
await save(false);
for (const height of [0, 40])
  for (const rotation of [0, 37, 90, 180]) {
    let document = {
      version: 1,
      map: "Selected building entrance",
      size: [1600, 3200],
      camera: { kind: "oblique-orthographic", elevation_deg: 35 },
      objects: [],
      groups: [],
      sceneAssets: [],
      assetSources: [],
      terrain: createTerrainGrid([0, 0, 1600, 3200], 400, 0),
    };
    for (const x of [450.25, 1150.25]) {
      const inserted = insertProjectionAsset(document, asset, reference, [x, 1800.75, height]);
      document = inserted.document;
      document.groups.find((group) => group.id === inserted.selection.id).transform.rot_deg =
        rotation;
    }
    const transform = (group, p) => {
      const part = document.objects.find(
        (entry) =>
          entry.group === group.id && entry.node === `asset:${asset.id}:${definition.node}`,
      );
      const authored = asset.parts.find((entry) => entry.node === definition.node);
      const matrix = partMatrix(document.camera, document, {
        ...part,
        obstacle: authored.obstacle_local_game,
      });
      return sceneToGame(
        document.camera,
        applyAffineMatrix(matrix, gameToScene(document.camera, ...p)),
      );
    };
    const terrainHeight = transform(document.groups[0], definition.outside)[2];
    for (const vertex of document.terrain.vertices) vertex.position[2] = terrainHeight;
    const compiled = compileMap(document, [0, 0, ...document.size], new Map([[asset.id, asset]]), {
      bestEffort: true,
    });
    const doors = compiled.descriptor.asset_geometry.buildings.flatMap(
      ({ Building }) => Building.doors,
    );
    const file = `${rotation}-${height}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
    const row = {
      file,
      rotation,
      height,
      terrainHeight,
      warnings: compiled.warnings,
      building_approaches: [],
    };
    results.push(row);
    await save(false);
    for (const group of document.groups) {
      const [x, y, z] = transform(group, definition.middle);
      const matches = doors.filter(
        (door) => door.point_mid[0] === Math.round(x) && door.point_mid[1] === Math.round(y - z),
      );
      assert.equal(matches.length, 1, `Selected entrance must bind: ${output}/${file}`);
      const door = matches[0];
      const dx = door.point_out[0] - door.point_mid[0],
        dy = door.point_out[1] - door.point_mid[1];
      const length = Math.hypot(dx, dy);
      assert.ok(length > 0);
      row.building_approaches.push({
        point_out: door.point_out,
        layer: door.layer_out,
        sector: door.sector_out,
        source_layer: door.layer_out,
        source_world: sourceLocal
          ? transform(group, [...sourceLocal, definition.outside[2]])
          : [
              door.point_out[0] + (dx / length) * 32,
              door.point_out[1] + (dy / length) * 32 + terrainHeight,
              terrainHeight,
            ],
      });
    }
    await save(false);
  }
await save(true);
