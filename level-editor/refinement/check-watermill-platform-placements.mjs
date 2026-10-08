import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { gameToScene } from "../shared/src/scene.ts";
import { sceneToGame, applyAffineMatrix } from "../shared/src/geometry.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";

const [stage] = process.argv.slice(2);
assert.ok(stage, "Provide a reviewed watermill gameplay stage");
const id = "leicester-watermill";
const source = await readStoredMap("library/scenes/leicester.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", source.assetSources, source.sceneAssets);
const edit = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8")).find(
  (e) => e.asset === id,
);
assert.equal(
  edit?.descriptorSha256,
  source.assetSources.find((s) => s.id === id)?.descriptor_sha256,
);
assets.get(id).gameplay = edit.gameplay;
const output = await fs.mkdtemp("work/map-compile/watermill-platform-placements-");
console.log(output);
const results = [];
const report = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      scope: "static-geometry-only-not-gameplay-parity",
      gameplayStage: stage,
      complete,
      results,
    }),
  );
await report(false);
for (const height of [0, 40])
  for (const rotation of [0, 37, 90, 180]) {
    const document = {
      version: 1,
      map: "Placed watermill platforms",
      camera: source.camera,
      size: [2400, 1800],
      objects: [],
      groups: [],
      sceneAssets: source.sceneAssets.filter((s) => s.id === id),
      assetSources: source.assetSources.filter((s) => s.id === id),
      terrain: createTerrainGrid([0, 0, 2400, 1800], 600, height),
    };
    const original = source.groups.find((g) => g.id === id);
    assert.ok(original);
    for (const copy of [0, 1]) {
      const group = structuredClone(original);
      group.id = `copy-${copy}`;
      group.transform = {
        dx: 600 + 1000 * copy,
        dy: 900,
        dz: original.transform.dz + height,
        rot_deg: rotation,
      };
      document.groups.push(group);
      document.objects.push(
        ...source.objects
          .filter((o) => o.group === id)
          .map((o) => ({
            ...structuredClone(o),
            id: `${group.id}/${o.id}`,
            group: group.id,
          })),
      );
    }
    const compiled = compileMap(document, [0, 0, ...document.size], assets, { bestEffort: true });
    const doors = compiled.descriptor.asset_geometry.buildings.flatMap(
      (b) => b.Building?.doors ?? [],
    );
    assert.equal(
      doors.length,
      6,
      compiled.warnings.filter((w) => /Door |Receiver |Jump /.test(w)).join("\n"),
    );
    assert.equal(
      compiled.descriptor.asset_geometry.jump_line_pairs?.length,
      2,
      compiled.warnings.filter((warning) => warning.startsWith("Jump ")).join("\n"),
    );
    const probes = document.groups.flatMap((group) =>
      edit.gameplay.interiors[0].doors.map((door) => {
        const object = document.objects.find(
          (o) => o.group === group.id && o.node === `asset:${id}:${door.node}`,
        );
        assert.ok(object);
        const world = sceneToGame(
          document.camera,
          applyAffineMatrix(
            partMatrix(document.camera, document, object),
            gameToScene(document.camera, ...door.outside),
          ),
        );
        const point = [Math.round(world[0]), Math.round(world[1] - world[2])];
        const matching = doors.filter((d) => d.point_out.every((n, i) => n === point[i]));
        assert.equal(matching.length, 1);
        return {
          id: `${group.id}/${door.id}`,
          source_world: world,
          point_out: point,
          layer: matching[0].layer_out,
          sector: matching[0].sector_out,
        };
      }),
    );
    const file = `watermill-${height}-${rotation}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
    const geometry = compiled.descriptor.asset_geometry;
    const jumpZoneMisses = geometry.jump_line_pairs.flatMap((pair, pairIndex) =>
      [
        [pair.line1, pair.line2],
        [pair.line2, pair.line1],
      ].flatMap(([line, other], side) => {
        const zone = geometry.jump_zones[other.jump_zone_index];
        return [line.point_a, line.point_b].flatMap((point, endpoint) =>
          pointInGameplayPolygon(point.slice(0, 2), zone.polygon.points, true)
            ? []
            : [{ pair: pairIndex, side, endpoint, point, zone: other.jump_zone_index }],
        );
      }),
    );
    results.push({
      map: file,
      file,
      warnings: compiled.warnings,
      building_approaches: probes,
      jumpZoneMisses,
    });
    const separated = structuredClone(document);
    separated.terrain = createTerrainGrid([0, 0, ...document.size], 600, height - 20);
    const disconnected = compileMap(separated, [0, 0, ...document.size], assets, {
      bestEffort: true,
    });
    const retained = disconnected.descriptor.asset_geometry.buildings.flatMap(
      (b) => b.Building?.doors ?? [],
    );
    assert.equal(retained.length, 2, "Displaced terrain must not support the four lower entrances");
    assert.equal(disconnected.descriptor.asset_geometry.jump_line_pairs?.length ?? 0, 0);
    results.at(-1).disconnectedGround = {
      retainedPlatformEntrances: retained.length,
      warnings: disconnected.warnings,
    };
    await report(false);
    console.log(`${file}: six entrances retained`);
  }
await report(true);
