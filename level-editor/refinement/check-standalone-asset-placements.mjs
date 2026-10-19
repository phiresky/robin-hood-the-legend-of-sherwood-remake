// Place an unreferenced library asset through the editor's actual insertion command.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";

const [stage, id] = process.argv.slice(2);
assert.ok(stage && id, "Supply gameplay stage and asset ID");
const edit = JSON.parse(await fs.readFile(`${stage}/edits.json`)).find((edit) => edit.asset === id);
assert.ok(edit);
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json"));
const entry = index.assets.find((entry) => entry.id === id);
assert.ok(entry);
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
assert.equal(hash(bytes), edit.descriptorSha256);
const model = await fs.readFile(`library/3d-assets/${entry.model}`);
assert.equal(hash(model), edit.modelSha256);
const descriptor = { ...JSON.parse(bytes), gameplay: edit.gameplay };
const reference = {
  id,
  descriptor: `3d-assets/${entry.descriptor}`,
  model: `3d-assets/${entry.model}`,
  descriptor_sha256: edit.descriptorSha256,
  model_sha256: edit.modelSha256,
  model_scene: descriptor.model_scene,
};
const output = await fs.mkdtemp("work/needs-review/map-compile/standalone-asset-placements-");
console.log(output);
const results = [];
const report = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      complete,
      scope: "editor-insertion-and-building-approaches-not-rendered-parity",
      gameplayStage: stage,
      asset: id,
      results,
    }),
  );
await report(false);
for (const height of [0, 40])
  for (const rotation of [0, 37, 90, 180]) {
    let document = {
      version: 1,
      map: `${id}-${rotation}-${height}`,
      size: [3000, 3000],
      camera: { kind: "oblique-orthographic", elevation_deg: 35 },
      groups: [],
      objects: [],
      sceneAssets: [],
      assetSources: [],
      terrain: createTerrainGrid([0, 0, 3000, 3000], 3000, height),
    };
    for (const x of [800, 1900]) {
      const inserted = insertProjectionAsset(document, descriptor, reference, [x, 1500, height]);
      document = inserted.document;
      document.groups.find((group) => group.id === inserted.selection.id).transform.rot_deg =
        rotation;
    }
    const compiled = compileMap(document, [0, 0, 3000, 3000], new Map([[id, descriptor]]), {
      bestEffort: true,
    });
    const geometry = compiled.descriptor.asset_geometry;
    const solids = geometry.sight_obstacles.filter((obstacle) => obstacle.solid);
    assert.equal(edit.gameplay.collision, "parts", "This audit counts part collision volumes");
    const authoredSolids = descriptor.parts.filter(
      (part) =>
        part.obstacle_local_game?.solid &&
        part.collision !== "none" &&
        part.mission_profile === undefined,
    );
    const authoredVolumes = (edit.gameplay.volumes ?? []).filter((volume) => volume.shape.solid);
    assert.equal(
      solids.length,
      2 * (authoredSolids.length + authoredVolumes.length),
      "Both copies must retain every solid sight volume, including those with separate movement contours",
    );
    const upperReceivers = geometry.sight_obstacles.filter(
      (receiver) => receiver.projection_area && receiver.points.every((point) => point.z_top > 500),
    );
    if (id === "nottingham-castle-watchtower") {
      assert.equal(upperReceivers.length, 2, "Each tower needs its own upper floor");
      assert.notEqual(
        upperReceivers[0].projection_area[0],
        upperReceivers[1].projection_area[0],
        "Separated tower floors must not share a navigation sector",
      );
      for (const receiver of upperReceivers) {
        for (const point of receiver.points) {
          assert.ok(
            Math.abs(point.z_top - (820.00104 + height)) < 0.001,
            "The upper floor must follow the placed elevation",
          );
        }
      }
    }
    const buildings = (geometry.buildings ?? []).flatMap((entry) =>
      entry.Building ? [entry.Building] : [],
    );
    const interiors = edit.gameplay.interiors ?? [];
    const expectedDoors = interiors.reduce((count, room) => count + room.doors.length, 0);
    assert.equal(buildings.length, 2 * interiors.length, "Copies must retain independent rooms");
    const doors = buildings.flatMap((building) => building.doors);
    assert.equal(
      doors.length,
      2 * expectedDoors,
      `All authored entrances must survive placement: ${compiled.warnings.join("; ")}`,
    );
    const probes = doors.map((door) => {
      const receivers = geometry.sight_obstacles.filter(
        (receiver) =>
          receiver.projection_area?.[0] === door.sector_out &&
          receiver.projection_area?.[1] === door.layer_out &&
          pointInGameplayPolygon(
            door.point_out,
            receiver.points.map((p) => [p.x, p.y - p.z_top]),
            true,
          ),
      );
      assert.ok(receivers.length, "Door approach needs a placed receiving surface");
      const plane = heightPlane(
        (
          receivers[0].projection_plane ??
          receivers[0].points.slice(0, 3).map((p) => [p.x, p.y, p.z_top])
        ).map(([x, y, z]) => [x, y - z, z]),
      );
      const z = planeHeight(plane, door.point_out);
      return {
        point_out: door.point_out,
        layer: door.layer_out,
        sector: door.sector_out,
        source_world: [door.point_out[0], door.point_out[1] + z, z],
      };
    });
    const file = `${rotation}-${height}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    await fs.writeFile(`${output}/${rotation}-${height}.editor.json`, JSON.stringify(document));
    results.push({
      file,
      rotation,
      height,
      collision_parts: solids.length,
      upper_floor_receivers: upperReceivers.length,
      building_approaches: probes,
      warnings: compiled.warnings,
    });
    await report(false);
  }
await report(true);
console.log(
  JSON.stringify({
    exports: results.length,
    approaches: results.reduce((n, r) => n + r.building_approaches.length, 0),
  }),
);
