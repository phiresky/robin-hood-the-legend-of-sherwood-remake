import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { normalizeGameplayStateViews } from "../shared/src/gameplay-state-views.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { gameToScene } from "../shared/src/scene.ts";
import { sceneToGame, applyAffineMatrix } from "../shared/src/geometry.ts";

// Identify authoring owners without adding diagnostic names to the runtime format.
const directory = process.argv[2];
assert.ok(directory, "Supply a completed saved-map export diagnostic directory");
const report = JSON.parse(await fs.readFile(`${directory}/diagnostics.json`, "utf8"));
assert.equal(report.complete, true);
const results = [];
const writeReport = (complete) =>
  fs.writeFile(
    `${directory}/building-owners.json`,
    JSON.stringify(
      { scope: "asset-door-coordinate-matches-not-traversal-proof", complete, results },
      null,
      2,
    ),
  );
await writeReport(false);
for (const result of report.results) {
  let document = await readStoredMap(`library/scenes/${result.map}.rhlos-map.json`, "library");
  let descriptors = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
  ({ document, descriptors } = normalizeGameplayStateViews(document, descriptors));
  const compiled = JSON.parse(await fs.readFile(`${directory}/${result.file}`, "utf8"));
  const placements = new Map();
  const hidden = new Set(document.groups.filter((group) => group.hidden).map((group) => group.id));
  for (const part of document.objects) {
    const match = /^asset:([^:]+):(.+)$/.exec(part.node);
    if (!match) continue;
    const descriptor = descriptors.get(match[1]);
    const id = `${part.group ?? part.id}/${match[1]}`;
    if (!placements.has(id))
      placements.set(id, { id, descriptor, parts: new Map(), visible: false });
    const instance = placements.get(id);
    instance.visible ||= !part.hidden && !hidden.has(part.group);
    const definition = descriptor.parts.find((candidate) => candidate.node === match[2]);
    assert.ok(definition, `Missing part ${part.node}`);
    instance.parts.set(match[2], { ...part, obstacle: definition.obstacle_local_game });
  }
  const candidates = [];
  for (const instance of placements.values()) {
    if (!instance.visible) continue;
    for (const room of instance.descriptor.gameplay?.interiors ?? [])
      for (const door of room.doors) {
        const part = instance.parts.get(door.node);
        assert.ok(part, `Missing door frame ${instance.id}/${door.node}`);
        const matrix = partMatrix(document.camera, document, part);
        const world = (point) => {
          const [x, y, z] = sceneToGame(
            document.camera,
            applyAffineMatrix(matrix, gameToScene(document.camera, ...point)),
          );
          return [x - result.bounds[0], y - result.bounds[1], z];
        };
        const project = (point) => {
          const [x, y, z] = world(point);
          return [Math.round(x), Math.round(y - z)];
        };
        candidates.push({
          asset: instance.descriptor.id,
          placement: instance.id,
          room: room.id,
          door: door.id,
          node: door.node,
          source_world: world(door.outside),
          point_out: project(door.outside),
          point_mid: project(door.middle),
          point_in: project(door.inside),
        });
      }
  }
  compiled.asset_geometry.buildings?.forEach(({ Building }, building) => {
    Building.doors.forEach((door, index) => {
      const matches = candidates.filter(
        (candidate) =>
          JSON.stringify(candidate.point_in) === JSON.stringify(door.point_in) &&
          JSON.stringify(candidate.point_mid) === JSON.stringify(door.point_mid),
      );
      assert.equal(matches.length, 1, `${result.map} building ${building} door ${index} ownership`);
      results.push({
        map: result.map,
        ...matches[0],
        building,
        index,
        blocked: door.passage_states?.some((state) => state.allowed_states.length === 0) ?? false,
      });
    });
  });
}
await writeReport(true);
console.log(
  `Matched ${results.length} doors; ${results.filter((result) => result.blocked).length} blocked`,
);
