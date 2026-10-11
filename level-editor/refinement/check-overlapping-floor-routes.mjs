import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { assetCompilerFixture } from "../shared/test-fixtures/asset-gameplay.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { gameToScene } from "../shared/src/scene.ts";
import { applyAffineMatrix, sceneToGame } from "../shared/src/geometry.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";
import { parseStoredMap, serializeStoredMap } from "../shared/src/index.ts";
import { compileMap, packageCompiledMap } from "../app/src/map-compile.ts";

const { unzipSync, strFromU8 } = createRequire(new URL("../app/package.json", import.meta.url))(
  "fflate",
);
const underDeck = process.argv.includes("--under-deck");
const output = await fs.mkdtemp(
  `work/map-compile/${underDeck ? "under-deck" : "overlapping-floor"}-routes-`,
);
console.log(output);
const results = [];
const report = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({ complete, scope: "overlapping-joined-floors", underDeck, results }),
  );
await report(false);
for (const rotation of [0, 37, 90, 180])
  for (const elevation of [0, 40]) {
    const { document, assets, hut } = assetCompilerFixture();
    const surface = (id, navigationRegion, x0, x1, z0, z1) => ({
      id,
      navigationRegion,
      node: "building-999",
      preserveMovementPrecision: true,
      polygon: [
        [x0, 100 + z0],
        [x1, 100 + z1],
        [x1, 200 + z1],
        [x0, 200 + z0],
      ],
      height: [z0, z1, z1, z0],
    });
    hut.gameplay.collision = "none";
    hut.gameplay.doors = [];
    hut.gameplay.surfaces = [
      surface("lower", "lower", 0, 200, 20, 20),
      surface("start", "upper", 300, 400, 20, 20),
      surface("ramp", "upper", 200, 300, 60, 20),
      surface("deck", "upper", 100, 200, 60, 60),
    ];
    if (underDeck)
      hut.gameplay.surfaces[0].polygon = [
        [100, 0],
        [200, 0],
        [200, 440],
        [100, 440],
      ];
    document.objects = document.objects.filter((o) => o.group);
    if (underDeck) Object.assign(document.objects[0].transform, { dx: 0, dy: 0 });
    document.groups[0].transform = { dx: 1100, dy: 900, dz: elevation, rot_deg: rotation };
    const group = structuredClone(document.groups[0]);
    group.id = "copy";
    group.transform.dx += 900;
    const copy = structuredClone(document.objects[0]);
    copy.id = "copy-body";
    copy.group = group.id;
    document.groups.push(group);
    document.objects.push(copy);
    document.size = [3000, 2000];
    document.map = `Overlapping floors ${rotation} ${elevation}`;
    const bounds = [0, 0, 3000, 2000];
    const compiled = compileMap(document, bounds, assets);
    const bytes = await packageCompiledMap(compiled, {
      color: new Uint8Array(3000 * 2000 * 4),
      depth: new Uint16Array(3000 * 2000),
    });
    const files = unzipSync(bytes);
    const read = (path) => {
      assert.ok(files[path]);
      return JSON.parse(strFromU8(files[path]));
    };
    const descriptor = read(`Data/Levels/${compiled.name}.level.json`);
    assert.deepEqual(descriptor, JSON.parse(JSON.stringify(compiled.descriptor)));
    const compileReport = read("compile-report.json");
    assert.deepEqual(compileReport.warnings, compiled.warnings);
    const stored = read(compileReport.editor_document);
    const reopened = parseStoredMap(stored, assets);
    assert.deepEqual(serializeStoredMap(reopened, assets), serializeStoredMap(document, assets));
    assert.deepEqual(
      JSON.parse(JSON.stringify(compileMap(reopened, bounds, assets).descriptor)),
      descriptor,
    );
    const file = `floors-${rotation}-${elevation}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(descriptor));
    await fs.writeFile(`${output}/${file}.zip`, bytes);
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(stored));
    for (const object of document.objects) {
      const matrix = partMatrix(document.camera, document, object);
      const world = (p) =>
        sceneToGame(document.camera, applyAffineMatrix(matrix, gameToScene(document.camera, ...p)));
      const lanes = underDeck
        ? [130, 170].map((x) =>
            [
              [x, 40, 20],
              [x, 400, 20],
            ].map(world),
          )
        : [130, 170].map((y) =>
            [
              [350, y + 20, 20],
              [150, y + 60, 60],
            ].map(world),
          );
      const goal = lanes[0][1];
      const projected = [goal[0], goal[1] - goal[2]];
      const layers = new Set(
        descriptor.asset_geometry.sight_obstacles
          .filter((o) => {
            if (!o.projection_area) return false;
            const points = o.points.map((p) => [p.x, p.y - p.z_top, p.z_top]);
            return (
              Math.abs(planeHeight(heightPlane(points), projected) - goal[2]) < 1e-4 &&
              pointInGameplayPolygon(
                projected,
                points.map((p) => [p[0], p[1]]),
              )
            );
          })
          .map((o) => o.projection_area[1]),
      );
      assert.equal(
        layers.size,
        1,
        `${file}/${object.id}: receiver missing at ${JSON.stringify(goal)}`,
      );
      results.push({
        file,
        rotation,
        elevation,
        copy: object.id,
        layer: [...layers][0],
        probes: lanes.map((lane) => lane.map((p) => [p[0], p[1] - p[2]])),
        warnings: compiled.warnings,
      });
    }
    await report(false);
  }
await report(true);
