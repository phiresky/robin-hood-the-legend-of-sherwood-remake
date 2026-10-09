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
for (const rotation of process.argv.includes("--rotated") ? [0, 37, 90, 180] : [0]) {
  const radians = (rotation * Math.PI) / 180;
  const direction = [Math.cos(radians), Math.sin(radians)];
  const start = [250 - 150 * direction[0], 250 - 150 * direction[1]];
  for (const elevation of [0, 40]) {
    for (const rise of [0, 30]) {
      for (const enabled of [false, true]) {
        const { document, asset, assets, bounds } = wallSplineFixture();
        const path = document.splines[0];
        path.points = [
          [...start, elevation],
          [250 + 150 * direction[0], 250 + 150 * direction[1], elevation + rise],
        ];
        for (const vertex of document.terrain.vertices)
          vertex.position[2] =
            elevation +
            (((vertex.position[0] - start[0]) * direction[0] +
              (vertex.position[1] - start[1]) * direction[1]) *
              rise) /
              300;
        if (enabled)
          asset.gameplay.movementClearances = [
            {
              id: "raised-cutout",
              node: "body",
              height: 30,
              navigationHeight: 0,
              polygon: [
                [-40, -80],
                [40, -80],
                [40, 80],
                [-40, 80],
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
        const first = sceneToGame(document.camera, curve.getPointAt(0).toArray());
        const last = sceneToGame(document.camera, curve.getPointAt(1).toArray());
        const dx = last[0] - first[0],
          dy = last[1] - last[2] - first[1] + first[2];
        const length = Math.hypot(dx, dy);
        const normal = [(-dy / length) * 20, (dx / length) * 20];
        const probes = [];
        for (let repeat = 0; repeat < 3; repeat++) {
          for (const [offset, opening] of [
            [-22, true],
            [0, false],
            [22, true],
            [45, false],
          ]) {
            const centre = curve.getPointAt((repeat * 100 + 50 + offset) / curve.getLength());
            const [x, y, z] = sceneToGame(document.camera, centre.toArray());
            probes.push({
              layer: 0,
              start: [x - normal[0], y - z - normal[1]],
              end: [x + normal[0], y - z + normal[1]],
              reachable: enabled && opening,
            });
          }
        }
        assert.equal(probes.length, 12);
        const file = `clearance-${rotation}-${elevation}-${rise}-${enabled}.level.json`;
        await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
        await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
        results.push({
          file,
          map: file,
          rotation,
          warnings: compiled.warnings,
          movement_probes: probes,
          route_probes: enabled
            ? probes
                .filter((probe) => probe.reachable)
                .flatMap((probe) => [
                  { ...probe, sector: 0, max_length: 45 },
                  { ...probe, start: probe.end, end: probe.start, sector: 0, max_length: 45 },
                ])
            : [],
        });
      }
    }
  }
}
await fs.writeFile(
  `${output}/diagnostics.json`,
  JSON.stringify({ scope: "static-geometry-only-not-gameplay-parity", complete: true, results }),
);
console.log(output);
