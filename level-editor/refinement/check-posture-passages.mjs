import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import {
  joinedPostureCompilerFixture,
  closedPostureCompilerFixture,
  alternatingPostureCompilerFixture,
} from "../shared/test-fixtures/asset-gameplay.ts";
import { compileMap, packageCompiledMap } from "../app/src/map-compile.ts";
import { serializeStoredMap, parseStoredMap } from "../shared/src/index.ts";

const { unzipSync, strFromU8 } = createRequire(new URL("../app/package.json", import.meta.url))(
  "fflate",
);
const options = process.argv.slice(2);
assert.ok(options.every((option) => ["--initially-closed", "--alternating"].includes(option)));
const initiallyClosed = options.includes("--initially-closed");
const alternating = options.includes("--alternating");
assert.ok(!(initiallyClosed && alternating), "choose one changing-obstacle scenario");
const output = await fs.mkdtemp(
  `work/needs-review/map-compile/${alternating ? "alternating-" : initiallyClosed ? "closed-" : ""}posture-passages-`,
);
console.log(output);
const results = [];
const report = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      complete,
      scope: "editable-posture-passage-zips-awaiting-native-traversal",
      results,
    }),
  );
await report(false);
for (const rotation of [0, 37, 90, 180])
  for (const elevation of [0, 40])
    for (const obstruction of initiallyClosed || alternating
      ? ["clear"]
      : ["clear", "split", "blocked"]) {
      const { document, assets, hut, upper } = joinedPostureCompilerFixture();
      if (initiallyClosed)
        hut.gameplay.movementTransitions =
          closedPostureCompilerFixture().hut.gameplay.movementTransitions;
      if (alternating)
        hut.gameplay.movementTransitions =
          alternatingPostureCompilerFixture().hut.gameplay.movementTransitions;
      if (obstruction !== "clear")
        for (const asset of [hut, upper])
          asset.gameplay.movementBlockers = [
            {
              id: "socket-wall",
              node: "building-999",
              height: 40,
              polygon:
                obstruction === "split"
                  ? [
                      [90, 40],
                      [110, 40],
                      [110, 60],
                      [90, 60],
                    ]
                  : [
                      [90, 0],
                      [110, 0],
                      [110, 100],
                      [90, 100],
                    ],
            },
          ];
      for (const group of document.groups)
        group.transform = { dx: 900, dy: 900, dz: elevation, rot_deg: rotation };
      const groups = structuredClone(document.groups),
        objects = structuredClone(document.objects.filter((object) => object.group));
      document.groups.push(
        ...groups.map((group) => ({
          ...group,
          id: `copy/${group.id}`,
          transform: { ...group.transform, dx: group.transform.dx + 700 },
        })),
      );
      document.objects.push(
        ...objects.map((object) => ({
          ...object,
          id: `copy/${object.id}`,
          group: `copy/${object.group}`,
        })),
      );
      document.size = [3000, 2000];
      const bounds = [0, 0, 3000, 2000];
      const compiled = compileMap(document, bounds, assets);
      const expectedDoors = alternating
        ? compiled.descriptor.asset_geometry.doors.length
        : obstruction === "clear"
          ? 2
          : obstruction === "split"
            ? 4
            : 0;
      if (alternating) assert.ok(expectedDoors >= 4, "each copy needs alternate openings");
      assert.ok(
        !compiled.warnings.some((warning) => warning.includes("clipped to the export frame")),
      );
      assert.equal(compiled.descriptor.asset_geometry.doors.length, expectedDoors);
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
      const file = `posture-${rotation}-${elevation}-${obstruction}.level.json`;
      await fs.writeFile(`${output}/${file}`, JSON.stringify(descriptor));
      await fs.writeFile(`${output}/${file}.zip`, bytes);
      await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(stored));
      results.push({
        file,
        map: file,
        rotation,
        height: 40 + elevation,
        obstruction,
        initially_closed: initiallyClosed,
        alternating,
        expected_doors: expectedDoors,
        warnings: compiled.warnings,
      });
      await report(false);
    }
await report(true);
