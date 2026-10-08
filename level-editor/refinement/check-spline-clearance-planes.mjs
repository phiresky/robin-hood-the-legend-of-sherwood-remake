import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { wallSplineFixture } from "../shared/test-fixtures/wall-spline.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { splineCurve } from "../shared/src/spline-sampling.ts";
import { sceneToGame } from "../shared/src/geometry.ts";

// Synthetic authored openings: navigation follows the floor, while the physical
// cutout is thirty units higher. No library or level records are read.
const output = await fs.mkdtemp("work/map-compile/spline-clearance-planes-");
const results = [];
for (const elevation of [0, 40]) {
  for (const rise of [0, 30]) {
    for (const enabled of [false, true]) {
      const { document, asset, assets, bounds } = wallSplineFixture();
      const path = document.splines[0];
      path.points = [
        [100, 200, elevation],
        [400, 200, elevation + rise],
      ];
      for (const vertex of document.terrain.vertices)
        vertex.position[2] = elevation + ((vertex.position[0] - 100) * rise) / 300;
      if (enabled)
        asset.gameplay.movementClearances = [
          {
            id: "raised-cutout",
            node: "body",
            height: 30,
            navigationHeight: 0,
            polygon: [
              [-20, -80],
              [20, -80],
              [20, 80],
              [-20, 80],
            ],
            holes: [
              [
                [-4, -60],
                [4, -60],
                [4, 60],
                [-4, 60],
              ],
            ],
          },
        ];
      const compiled = compileMap(document, bounds, assets);
      const curve = splineCurve(path, document.camera);
      const probes = [];
      for (let repeat = 0; repeat < 3; repeat++) {
        for (const [offset, opening] of [
          [-12, true],
          [0, false],
          [12, true],
          [30, false],
        ]) {
          const centre = curve.getPointAt((repeat * 100 + 50 + offset) / curve.getLength());
          const [x, y, z] = sceneToGame(document.camera, centre.toArray());
          probes.push({
            layer: 0,
            start: [x, y - z - 20],
            end: [x, y - z + 20],
            reachable: enabled && opening,
          });
        }
      }
      assert.equal(probes.length, 12);
      const file = `clearance-${elevation}-${rise}-${enabled}.level.json`;
      await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
      await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
      results.push({ file, map: file, warnings: compiled.warnings, movement_probes: probes });
    }
  }
}
await fs.writeFile(
  `${output}/diagnostics.json`,
  JSON.stringify({ scope: "static-geometry-only-not-gameplay-parity", complete: true, results }),
);
console.log(output);
