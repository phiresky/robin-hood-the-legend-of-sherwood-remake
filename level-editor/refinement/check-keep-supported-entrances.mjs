// Author surrounding terrain for the keep's elevated exterior entrances.
import fs from "node:fs/promises";
import path from "node:path";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { gameToScene } from "../shared/src/scene.ts";
import { sceneToGame } from "../shared/src/geometry.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { serializeStoredMap } from "../shared/src/stored-level.ts";

const candidate = process.argv[2];
assert.ok(candidate, "Supply a combined keep model candidate directory");
const descriptor = JSON.parse(await fs.readFile(path.join(candidate, "asset.json")));
const authoring = JSON.parse(await fs.readFile(path.join(candidate, "authoring.json")));
const rotations = process.argv[3] ? [Number(process.argv[3])] : [0, 37, 90, 180];
const output = await fs.mkdtemp("work/needs-review/map-compile/keep-supported-entrances-");
console.log(output);
await fs.mkdir(path.join(output, "scenes"));
for (const directory of ["3d-assets", "game-data", "population"])
  await fs.symlink(path.resolve(candidate, directory), path.join(output, directory), "dir");
const results = [];
const save = (complete) =>
  fs.writeFile(
    path.join(output, "diagnostics.json"),
    JSON.stringify({
      scope: "authored-terrain-building-approaches-not-full-keep-parity",
      complete,
      results,
    }),
  );
await save(false);
for (const rotation of rotations) {
  const document = JSON.parse(
    await fs.readFile(path.join(authoring.candidate, `${rotation}-40.level.json.scene.json`)),
  );
  assert.equal(document.assetSources.length, 1);
  document.assetSources[0].descriptor_sha256 = createHash("sha256")
    .update(JSON.stringify(descriptor))
    .digest("hex");
  document.assetSources[0].model_sha256 = authoring.modelSha256;
  document.map = `keep-supported-${rotation}`;
  const supports = descriptor.gameplay.interiors
    .filter((interior) => ["interior-1", "interior-4"].includes(interior.id))
    .map((interior) => {
      const door = interior.doors.find((door) => door.id.endsWith("/door-0"));
      const part = document.objects.find((part) => part.node.endsWith(door.node));
      assert.ok(part);
      const matrix = partMatrix(document.camera, document, part);
      const local = gameToScene(document.camera, ...door.outside);
      const world = sceneToGame(
        document.camera,
        [0, 1, 2].map(
          (row) =>
            matrix[row] * local[0] +
            matrix[row + 4] * local[1] +
            matrix[row + 8] * local[2] +
            matrix[row + 12],
        ),
      );
      return { door: door.id, center: world };
    });
  // These plateaus and their connecting slopes are ordinary editor terrain,
  // explicitly authored for this assembly fixture, not compiler-created floors.
  document.terrain = createTerrainGrid([0, 0, 2400, 2400], 60, 40);
  for (const vertex of document.terrain.vertices) {
    vertex.position[2] = Math.max(
      40,
      ...supports.map(({ center }) => {
        const distance = Math.max(
          Math.abs(vertex.position[0] - center[0]),
          Math.abs(vertex.position[1] - center[1]),
        );
        const weight = Math.max(0, Math.min(1, (300 - distance) / 180));
        return 40 + (center[2] - 40) * weight;
      }),
    );
  }
  const compiled = compileMap(
    document,
    [0, 0, 2400, 2400],
    new Map([[descriptor.id, descriptor]]),
    { bestEffort: true },
  );
  const geometry = compiled.descriptor.asset_geometry;
  const doors = geometry.buildings.flatMap((entry) => entry.Building?.doors ?? []);
  assert.equal(doors.length, 12, "Every authored interior entrance must bind");
  assert.equal(geometry.movement_transitions.length, 2, "Both independent controls must bind");
  const approaches = doors.map((door) => {
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
    assert.ok(receiver, "Every compiled entrance retains a receiving plane");
    const plane = heightPlane(
      (
        receiver.projection_plane ?? receiver.points.slice(0, 3).map((p) => [p.x, p.y, p.z_top])
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
  const file = `${rotation}-40.level.json`;
  await fs.writeFile(path.join(output, file), JSON.stringify(compiled.descriptor));
  await fs.writeFile(path.join(output, `${file}.scene.json`), JSON.stringify(document));
  await fs.writeFile(
    path.join(output, `scenes/${document.map}.rhlos-map.json`),
    JSON.stringify(serializeStoredMap(document, new Map([[descriptor.id, descriptor]]))),
  );
  results.push({ file, supports, building_approaches: approaches, warnings: compiled.warnings });
  console.log(
    JSON.stringify({
      rotation,
      doors: doors.length,
      controls: geometry.movement_transitions.length,
      warnings: compiled.warnings.length,
    }),
  );
  await save(false);
}
await save(true);
