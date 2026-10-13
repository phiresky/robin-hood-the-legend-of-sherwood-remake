// Reconnect separate hall/tower assets through their placed receiving geometry.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { sceneToGame } from "../shared/src/geometry.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";

const stage = process.argv[2];
assert.ok(stage, "Supply the standalone hall gameplay candidate");
const [edit] = JSON.parse(await fs.readFile(`${stage}/edits.json`));
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const read = async (id) => {
  const directory = `3d-assets/nottingham/${id}`;
  const bytes = await fs.readFile(`library/${directory}/asset.json`);
  const descriptor = JSON.parse(bytes);
  const model = await fs.readFile(`library/${directory}/${descriptor.model}`);
  return {
    descriptor,
    reference: {
      id,
      descriptor: `${directory}/asset.json`,
      model: `${directory}/${descriptor.model}`,
      descriptor_sha256: hash(bytes),
      model_sha256: hash(model),
      model_scene: descriptor.model_scene,
    },
  };
};
const hall = await read(edit.asset),
  tower = await read("nottingham-castle-watchtower");
assert.equal(hall.reference.descriptor_sha256, edit.descriptorSha256);
assert.equal(hall.reference.model_sha256, edit.modelSha256);
hall.descriptor.gameplay = edit.gameplay;
if (process.argv[3]) {
  const edits = JSON.parse(await fs.readFile(`${process.argv[3]}/edits.json`));
  const towerEdit = edits.find((row) => row.asset === tower.descriptor.id);
  assert.equal(towerEdit.asset, tower.descriptor.id);
  assert.equal(towerEdit.descriptorSha256, tower.reference.descriptor_sha256);
  assert.equal(towerEdit.modelSha256, tower.reference.model_sha256);
  tower.descriptor.gameplay = towerEdit.gameplay;
  const hallEdit = edits.find((row) => row.asset === hall.descriptor.id);
  if (hallEdit) {
    assert.equal(hallEdit.descriptorSha256, hall.reference.descriptor_sha256);
    assert.equal(hallEdit.modelSha256, hall.reference.model_sha256);
    hall.descriptor.gameplay = hallEdit.gameplay;
  }
}
const assets = new Map([hall, tower].map(({ descriptor }) => [descriptor.id, descriptor]));
const offset = tower.descriptor.source_origin_scene.map(
  (v, i) => v - hall.descriptor.source_origin_scene[i],
);
const output = await fs.mkdtemp("work/needs-review/map-compile/hall-tower-connections-");
console.log(output);
const results = [];
const save = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      scope: "separate-assets-upper-entrance-connections-not-all-hall-entrances",
      complete,
      results,
    }),
  );
await save(false);
for (const height of [0, 40])
  for (const rotation of [0, 37, 90, 180]) {
    let document = JSON.parse(
      await fs.readFile(`${stage}/${rotation}-${height}.level.json.scene.json`),
    );
    const halls = [...document.groups];
    for (const group of halls) {
      const hallPart = document.objects.find((part) => part.group === group.id);
      const hallMatrix = partMatrix(document.camera, document, hallPart);
      const inserted = insertProjectionAsset(
        document,
        tower.descriptor,
        tower.reference,
        [0, 0, 0],
      );
      document = inserted.document;
      const towerGroup = document.groups.find((row) => row.id === inserted.selection.id);
      towerGroup.transform.rot_deg = rotation;
      const towerPart = document.objects.find((part) => part.group === towerGroup.id);
      const towerMatrix = partMatrix(document.camera, document, towerPart);
      const desired = [0, 1, 2].map(
        (row) =>
          hallMatrix[row] * offset[0] +
          hallMatrix[row + 4] * offset[1] +
          hallMatrix[row + 8] * offset[2] +
          hallMatrix[row + 12],
      );
      const translation = sceneToGame(
        document.camera,
        desired.map((v, i) => v - towerMatrix[12 + i]),
      );
      for (const [i, key] of ["dx", "dy", "dz"].entries())
        towerGroup.transform[key] += translation[i];
      const actual = partMatrix(document.camera, document, towerPart);
      for (let i = 0; i < 16; i++)
        assert.ok(
          Math.abs(actual[i] - (i >= 12 && i <= 14 ? desired[i - 12] : hallMatrix[i])) < 1e-6,
        );
    }
    const compiled = compileMap(document, [0, 0, 3000, 3000], assets, { bestEffort: true });
    const geometry = compiled.descriptor.asset_geometry;
    const approaches = [];
    for (const building of geometry.buildings)
      for (const door of building.Building?.doors ?? []) {
        const receiver = geometry.sight_obstacles.find(
          (s) =>
            s.projection_area?.[0] === door.sector_out &&
            s.projection_area?.[1] === door.layer_out &&
            pointInGameplayPolygon(
              door.point_out,
              s.points.map((p) => [p.x, p.y - p.z_top]),
              true,
            ),
        );
        assert.ok(receiver);
        const plane = heightPlane(
          (
            receiver.projection_plane ?? receiver.points.slice(0, 3).map((p) => [p.x, p.y, p.z_top])
          ).map(([x, y, z]) => [x, y - z, z]),
        );
        const z = planeHeight(plane, door.point_out);
        approaches.push({
          point_out: door.point_out,
          layer: door.layer_out,
          sector: door.sector_out,
          source_world: [door.point_out[0], door.point_out[1] + z, z],
        });
      }
    assert.equal(
      approaches.filter((probe) => probe.source_world[2] > 800).length,
      2,
      "Both separate towers must support their hall entrance",
    );
    const file = `${rotation}-${height}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
    results.push({
      file,
      height,
      rotation,
      building_approaches: approaches,
      warnings: compiled.warnings,
    });
    await save(false);
  }
await save(true);
