// Reassemble reusable keep/turret assets with their separate supporting annex.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";
import {
  partMatrix,
  groupCentroid,
  groupParts,
  gameTransformMatrix,
} from "../shared/src/level3d.ts";
import { gameToScene } from "../shared/src/scene.ts";
import { sceneToGame, applyAffineMatrix } from "../shared/src/geometry.ts";

const [stage, fixturePath] = process.argv.slice(2);
assert.ok(stage, "Supply the keep/turret doorway stage");
const fixture = fixturePath ? JSON.parse(await fs.readFile(fixturePath)) : undefined;
if (fixture) {
  assert.ok(Array.isArray(fixture.assets) && fixture.assets.length > 1);
  assert.equal(new Set(fixture.assets).size, fixture.assets.length);
  assert.ok(Number.isFinite(fixture.terrainHeight));
  assert.ok(
    fixture.missingEntrances === undefined || typeof fixture.missingEntrances === "boolean",
  );
  assert.ok(Array.isArray(fixture.entrances));
  assert.ok(fixture.entrances.length || fixture.lifts?.length);
  if (fixture.lifts) {
    assert.ok(Array.isArray(fixture.lifts));
    for (const lift of fixture.lifts)
      assert.ok(fixture.assets.includes(lift.asset) && typeof lift.id === "string");
  }
  assert.ok(fixture.missingLifts === undefined || typeof fixture.missingLifts === "boolean");
  for (const entrance of fixture.entrances) {
    assert.ok(fixture.assets.includes(entrance.asset) && typeof entrance.door === "string");
    assert.ok(entrance.sourceLocal?.length === 2 && entrance.sourceLocal.every(Number.isFinite));
  }
}
const ids = fixture?.assets ?? [
  "lincoln-keep",
  "lincoln-keep-annex",
  "lincoln-keep-annex-cone-turret",
];
const stored = JSON.parse(await fs.readFile("library/scenes/lincoln.rhlos-map.json"));
const references = stored.assetSources.filter((ref) => ids.includes(ref.id));
assert.equal(references.length, ids.length);
const assets = await pinnedDescriptors("library", references);
for (const edit of JSON.parse(await fs.readFile(`${stage}/edits.json`))) {
  const ref = references.find((ref) => ref.id === edit.asset);
  if (fixture && !ref) continue;
  assert.equal(ref?.descriptor_sha256, edit.descriptorSha256);
  if (edit.modelSha256 !== undefined) assert.equal(ref?.model_sha256, edit.modelSha256);
  assets.get(edit.asset).gameplay = edit.gameplay;
}
const placements = ids.map((id) => {
  const matches = stored.placements.filter(
    (placement) => placement.assets.length === 1 && placement.assets[0] === id,
  );
  assert.equal(matches.length, 1);
  assert.equal(matches[0].transform.rot_deg, 0);
  return matches[0];
});
const origin = placements[0].transform;
const output = await fs.mkdtemp(
  `work/needs-review/map-compile/${fixture ? "selected-neighbor" : "keep-neighbor"}-placements-`,
);
console.log(output);
const results = [];
const save = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      scope: fixture
        ? "selected-neighbor-entrances-placement-check-not-full-asset-parity"
        : "copied-rotated-raised-keep-and-separate-annex-awaiting-native-verification",
      complete,
      stage,
      ...(fixture ? { fixture } : {}),
      results,
    }),
  );
await save(false);
for (const height of [0, 40])
  for (const rotation of [0, 37, 90, 180]) {
    let document = {
      version: 1,
      map: fixture ? "Selected neighboring assets" : "Keep and neighboring annex",
      size: [3200, 3200],
      camera: stored.camera,
      objects: [],
      groups: [],
      sceneAssets: [],
      assetSources: [],
      terrain: createTerrainGrid(
        [0, 0, 3200, 3200],
        400,
        (fixture?.terrainHeight ?? 472.00104) + height,
      ),
    };
    const radians = (rotation * Math.PI) / 180,
      c = Math.cos(radians),
      s = Math.sin(radians);
    const sinElevation = Math.sin((document.camera.elevation_deg * Math.PI) / 180);
    const groups = [];
    for (const x of [650.25, 2250.25])
      for (const placement of placements) {
        const asset = assets.get(placement.assets[0]);
        const dx = placement.transform.dx - origin.dx,
          dy = placement.transform.dy - origin.dy;
        const inserted = insertProjectionAsset(
          document,
          asset,
          references.find((ref) => ref.id === asset.id),
          [x + c * dx - (s * dy) / sinElevation, 1800.75 + s * dx * sinElevation + c * dy, height],
        );
        document = inserted.document;
        const group = document.groups.find((group) => group.id === inserted.selection.id);
        group.transform.dz = placement.transform.dz + height;
        group.transform.rot_deg = rotation;
        // Group turns use each asset's footprint centroid. Compensate that local
        // pivot so all neighboring parts undergo one rigid ground-plane rotation.
        const offset = sceneToGame(
          document.camera,
          applyAffineMatrix(
            gameTransformMatrix(
              document.camera,
              { dx: 0, dy: 0, dz: 0, rot_deg: rotation },
              groupCentroid(groupParts(document, group.id)),
            ),
            gameToScene(document.camera, 0, 0, 0),
          ),
        );
        group.transform.dx -= offset[0];
        group.transform.dy -= offset[1];
        groups.push({ group, asset, placement, clusterX: x });
      }
    const compiled = compileMap(document, [0, 0, ...document.size], assets, { bestEffort: true });
    const doors = (compiled.descriptor.asset_geometry.buildings ?? []).flatMap(
      ({ Building }) => Building.doors,
    );
    const file = `${rotation}-${height}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
    const row = {
      file,
      rotation,
      height,
      warnings: compiled.warnings,
      building_approaches: [],
      building_pairs: [],
      ...(fixture?.lifts ? { checked_lifts: 0 } : {}),
      ...(fixture?.missingEntrances ? { rejected_entrances: 0 } : {}),
    };
    results.push(row);
    await save(false);
    if (!fixture)
      assert.equal(
        doors.length,
        10,
        `Every keep and turret entrance must bind in ${output}/${file}`,
      );
    for (const { group, asset, placement, clusterX } of groups) {
      const transform = (node, p) => {
        const part = document.objects.find(
          (part) => part.group === group.id && part.node === `asset:${asset.id}:${node}`,
        );
        const definition = asset.parts.find((part) => part.node === node);
        const matrix = partMatrix(document.camera, document, {
          ...part,
          obstacle: definition.obstacle_local_game,
        });
        return sceneToGame(
          document.camera,
          applyAffineMatrix(matrix, gameToScene(document.camera, ...p)),
        );
      };
      for (const selected of fixture?.lifts?.filter((lift) => lift.asset === asset.id) ?? []) {
        const definition = asset.gameplay.lifts.find((lift) => lift.id === selected.id);
        assert.ok(definition, `Missing authored lift ${asset.id}/${selected.id}`);
        const expected = definition.doors.map((door) => {
          const [x, y, z] = transform(door.node, door.middle);
          return [Math.round(x), Math.round(y - z)];
        });
        const matches = (compiled.descriptor.asset_geometry.lifts ?? []).filter(
          (lift) =>
            lift.lift_type === definition.type &&
            expected.every((point) =>
              lift.doors.some((door) => door.point_mid.every((value, i) => value === point[i])),
            ),
        );
        assert.equal(
          matches.length,
          fixture.missingLifts ? 0 : 1,
          `Selected lift ${asset.id}/${selected.id} must ${fixture.missingLifts ? "remain unavailable" : "bind"} in ${file}`,
        );
        row.checked_lifts++;
      }
      for (const room of asset.gameplay.interiors ?? []) {
        const selected = room.doors.filter(
          (door) =>
            !fixture ||
            fixture.entrances.some(
              (entrance) => entrance.asset === asset.id && entrance.door === door.id,
            ),
        );
        const matches = selected.map((definition) => {
          const [x, y, z] = transform(definition.node, definition.middle);
          const found = doors.filter(
            (door) =>
              door.point_mid[0] === Math.round(x) && door.point_mid[1] === Math.round(y - z),
          );
          if (fixture?.missingEntrances) {
            assert.equal(found.length, 0, "Unsupported entrance must not bind to another floor");
            row.rejected_entrances++;
            return undefined;
          }
          assert.equal(found.length, 1);
          const door = found[0];
          const dx = door.point_out[0] - door.point_mid[0],
            dy = door.point_out[1] - door.point_mid[1];
          const length = Math.hypot(dx, dy);
          const outside = transform(definition.node, definition.outside);
          const rx = definition.outside[0] + placement.transform.dx - origin.dx;
          const ry = definition.outside[1] + placement.transform.dy - origin.dy;
          const expected = [
            clusterX + c * rx - (s * ry) / sinElevation,
            1800.75 + s * rx * sinElevation + c * ry,
            definition.outside[2] + placement.transform.dz + height,
          ];
          assert.ok(
            outside.every((value, i) => Math.abs(value - expected[i]) < 1e-6),
            "Separate group pivots must preserve the authored neighboring assembly",
          );
          const altitude = outside[2];
          const authoredSource = fixture?.entrances.find(
            (entrance) => entrance.asset === asset.id && entrance.door === definition.id,
          )?.sourceLocal;
          const source = authoredSource
            ? transform(definition.node, [...authoredSource, definition.outside[2]])
            : asset.id === "lincoln-keep" && definition.id === "interior-2/door-0"
              ? transform(definition.node, [70, 5, definition.outside[2]])
              : [
                  door.point_out[0] + (dx / length) * 32,
                  door.point_out[1] + (dy / length) * 32 + altitude,
                  altitude,
                ];
          row.building_approaches.push({
            point_out: door.point_out,
            layer: door.layer_out,
            sector: door.sector_out,
            source_layer: door.layer_out,
            source_world: source,
          });
          return door.point_out;
        });
        for (const from of matches.filter(Boolean))
          for (const to of matches.filter(Boolean))
            if (from !== to) row.building_pairs.push({ from, to, connected: true });
      }
    }
    if (fixture)
      assert.equal(
        fixture.missingEntrances ? row.rejected_entrances : row.building_approaches.length,
        fixture.entrances.length * 2,
      );
    if (fixture?.lifts) assert.equal(row.checked_lifts, fixture.lifts.length * 2);
    await save(false);
  }
await save(true);
