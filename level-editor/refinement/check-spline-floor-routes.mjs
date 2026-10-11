import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { wallPointMaskFloorFixture } from "../shared/test-fixtures/wall-spline.ts";
import { splineCurve } from "../shared/src/spline-sampling.ts";
import { sceneToGame } from "../shared/src/geometry.ts";
import { parseStoredMap, serializeStoredMap } from "../shared/src/index.ts";
import { compileMap, packageCompiledMap } from "../app/src/map-compile.ts";

const { unzipSync, strFromU8 } = createRequire(new URL("../app/package.json", import.meta.url))(
  "fflate",
);
const output = await fs.mkdtemp("work/map-compile/spline-mask-floor-routes-");
console.log(output);
const results = [];
const report = (complete) =>
  fs.writeFile(`${output}/diagnostics.json`, JSON.stringify({ complete, results }));
await report(false);
for (const curved of [false, true])
  for (const rise of [0, 30]) {
    const f = wallPointMaskFloorFixture(curved, rise);
    const compiled = compileMap(f.document, f.bounds, f.assets);
    const bytes = await packageCompiledMap(compiled, {
      color: new Uint8Array(500 * 500 * 4),
      depth: new Uint16Array(500 * 500),
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
    const reopened = parseStoredMap(stored, f.assets);
    assert.deepEqual(
      serializeStoredMap(reopened, f.assets),
      serializeStoredMap(f.document, f.assets),
    );
    assert.deepEqual(
      JSON.parse(JSON.stringify(compileMap(reopened, f.bounds, f.assets).descriptor)),
      descriptor,
    );
    const curve = splineCurve(f.document.splines[0], f.document.camera);
    const probes = [-22, 22].map((offset) =>
      [0.1, 0.9].map((t) => {
        const p = curve.getPointAt(t),
          d = curve.getTangentAt(t);
        const length = Math.hypot(d.x, d.y);
        const world = sceneToGame(f.document.camera, [
          p.x - (d.y / length) * offset,
          p.y + (d.x / length) * offset,
          p.z,
        ]);
        return [world[0], world[1] - world[2]];
      }),
    );
    assert.equal(descriptor.asset_geometry.masks.length, 3);
    const file = `floor-${curved}-${rise}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(descriptor));
    await fs.writeFile(`${output}/${file}.zip`, bytes);
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(stored));
    results.push({
      file,
      curved,
      rise,
      layer: descriptor.asset_geometry.masks[0].layer,
      probes,
      warnings: compiled.warnings,
    });
    await report(false);
  }
await report(true);
