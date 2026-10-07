import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { groupCentroid, partMatrix } from "../shared/src/level3d.ts";
import { gameToScene } from "../shared/src/scene.ts";
import { sceneToGame } from "../shared/src/geometry.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";
import { compileMap } from "../app/src/map-compile.ts";

const [stage] = process.argv.slice(2);
assert.ok(stage, "Provide the reviewed doorstep stage");
const source = await readStoredMap("library/scenes/york.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", source.assetSources, source.sceneAssets);
const ids = ["york-west-town-lane-stone-tower", "york-west-town-lane-narrow-timber-house"];
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
for (const id of ids) {
  const edit = edits.find((edit) => edit.asset === id);
  assert.equal(
    edit?.descriptorSha256,
    source.assetSources.find((entry) => entry.id === id)?.descriptor_sha256,
  );
  assets.get(id).gameplay = edit.gameplay;
}
const output = await fs.mkdtemp("work/map-compile/receiver-floor-placements-");
console.log(output);
const results = [],
  rejected = [];
const report = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      scope: "compiler-door-and-terrain-connections-not-native-traversal",
      complete,
      gameplayStage: stage,
      results,
    }),
  );
await report(false);
for (const height of [0, 40])
  for (const rotation of [0, 37, 90, 180]) {
    const size = [3000, 3000];
    const document = {
      version: 1,
      map: "receiver-floors",
      camera: source.camera,
      size,
      objects: [],
      groups: [],
      sceneAssets: source.sceneAssets.filter((entry) => ids.includes(entry.id)),
      assetSources: source.assetSources.filter((entry) => ids.includes(entry.id)),
      terrain: createTerrainGrid([0, 0, ...size], 1000, 90.00101 + height),
    };
    const radians = (rotation * Math.PI) / 180,
      sinT = Math.sin((source.camera.elevation_deg * Math.PI) / 180);
    const rotate = ([x, y]) => [
      x * Math.cos(radians) - (y * Math.sin(radians)) / sinT,
      x * Math.sin(radians) * sinT + y * Math.cos(radians),
    ];
    for (const [index, id] of ids.entries())
      for (const copy of [0, 1]) {
        const group = structuredClone(source.groups.find((group) => group.id === id));
        assert.ok(group && group.transform.rot_deg === 0);
        const objects = source.objects.filter((object) => object.group === id);
        const pivot = groupCentroid(objects),
          rotated = rotate(pivot);
        group.id = `copy${copy}/${id}`;
        group.transform = {
          dx: 800 + copy * 1400 - pivot[0] + rotated[0],
          dy: 800 + index * 1400 - pivot[1] + rotated[1],
          dz: group.transform.dz + height,
          rot_deg: rotation,
        };
        document.groups.push(group);
        document.objects.push(
          ...objects.map((object) => ({
            ...structuredClone(object),
            id: `copy${copy}/${object.id}`,
            group: group.id,
          })),
        );
      }
    const compile = (document) =>
      compileMap(document, [0, 0, ...size], assets, { bestEffort: true });
    const compiled = compile(document);
    const unbound = (result) =>
      result.warnings.filter(
        (warning) =>
          warning.startsWith("Navigation region ") && warning.includes("-receiver-physical-floor:"),
      );
    assert.equal(
      compiled.descriptor.asset_geometry.buildings.length,
      4,
      compiled.warnings.join("\n"),
    );
    // Each four-sided doorstep exposes one lower edge to independently authored ground.
    assert.equal(unbound(compiled).length, 12, unbound(compiled).join("\n"));
    for (const group of document.groups)
      assert.equal(
        unbound(compiled).filter((warning) => warning.includes(`Navigation region ${group.id}/`))
          .length,
        3,
        `${group.id}: expected exactly one terrain connection`,
      );
    const file = `receiver-floors-${height}-${rotation}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
    const buildingApproaches = document.groups.map((group) => {
      const id = ids.find((id) => group.id.endsWith(`/${id}`));
      const gameplay = assets.get(id).gameplay;
      const floor = gameplay.surfaces.find((surface) =>
        surface.id.endsWith("-receiver-physical-floor"),
      );
      const transform = (node, point) => {
        const object = document.objects.find(
          (object) => object.group === group.id && object.node === `asset:${id}:${node}`,
        );
        assert.ok(object);
        const matrix = partMatrix(document.camera, document, object);
        const local = gameToScene(document.camera, ...point);
        return sceneToGame(
          document.camera,
          [0, 1, 2].map(
            (row) =>
              matrix[row] * local[0] +
              matrix[4 + row] * local[1] +
              matrix[8 + row] * local[2] +
              matrix[12 + row],
          ),
        );
      };
      const points = floor.polygon.map(([x, y], index) =>
        transform(floor.node, [x, y, floor.height[index]]),
      );
      const bottom = Math.min(...points.map((point) => point[2]));
      const edge = points
        .map((point, index) => [point, points[(index + 1) % points.length]])
        .filter((edge) => edge.every((point) => Math.abs(point[2] - bottom) < 1e-5));
      assert.equal(edge.length, 1, `${group.id}: expected one level approach edge`);
      const [a, b] = edge[0];
      const midpoint = [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2];
      const center = [0, 1].map(
        (axis) => points.reduce((sum, point) => sum + point[axis], 0) / points.length,
      );
      let normal = [a[1] - b[1], b[0] - a[0]];
      if (normal[0] * (center[0] - midpoint[0]) + normal[1] * (center[1] - midpoint[1]) > 0)
        normal = normal.map((value) => -value);
      const length = Math.hypot(...normal);
      const source = [
        midpoint[0] + (normal[0] * 24) / length,
        midpoint[1] + (normal[1] * 24) / length,
        90.00101 + height,
      ];
      const door = gameplay.interiors[0].doors[0];
      const outside = transform(door.node, door.outside);
      const pointOut = [Math.round(outside[0]), Math.round(outside[1] - outside[2])];
      const matches = compiled.descriptor.asset_geometry.buildings
        .flatMap((building) => building.Building?.doors ?? [])
        .filter((door) => door.point_out.every((value, index) => value === pointOut[index]));
      assert.equal(matches.length, 1, `${group.id}: ambiguous compiled entrance`);
      return {
        id: `${group.id}/${door.id}`,
        source_world: source,
        point_out: pointOut,
        layer: matches[0].layer_out,
        sector: matches[0].sector_out,
      };
    });
    results.push({
      map: file,
      file,
      warnings: compiled.warnings,
      building_approaches: buildingApproaches,
    });
    for (const kind of ["missing", "raised"]) {
      const changed = structuredClone(document);
      if (kind === "missing") delete changed.terrain;
      else for (const vertex of changed.terrain.vertices) vertex.position[2] += 20;
      const invalid = compile(changed);
      assert.equal(unbound(invalid).length, 16, `${file}: ${kind} ground still connects`);
      for (const group of document.groups)
        assert.equal(
          unbound(invalid).filter((warning) => warning.includes(`Navigation region ${group.id}/`))
            .length,
          4,
          `${group.id}: disconnected terrain still matches a socket`,
        );
      rejected.push({ file, kind });
    }
    await report(false);
  }
await fs.writeFile(`${output}/rejected.json`, JSON.stringify(rejected));
await report(true);
console.log(
  JSON.stringify({
    output,
    exports: results.length,
    placements: results.length * 4,
    rejected: rejected.length * 4,
  }),
);
