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
const curved = process.argv.includes("--curved");
const deck = process.argv.includes("--deck");
assert.ok(!deck || curved, "--deck requires --curved");
for (const rotation of process.argv.includes("--rotated") ? [0, 37, 90, 180] : [0]) {
  const radians = (rotation * Math.PI) / 180;
  const direction = [Math.cos(radians), Math.sin(radians)];
  const start = [250 - 150 * direction[0], 250 - 150 * direction[1]];
  for (const elevation of deck ? [40, 80] : [0, 40]) {
    // Curved terrain cases use a flat floor; raised decks supply their own rising floor.
    for (const rise of deck ? [30] : curved ? [0] : [0, 30]) {
      for (const enabled of [false, true]) {
        const { document, asset, assets, bounds } = wallSplineFixture();
        const sine = Math.sin((document.camera.elevation_deg * Math.PI) / 180);
        // Cross-sections are perpendicular in scene space. Express the ground
        // gradient in game coordinates so it is constant across each section.
        const gradient = [direction[0], direction[1] / (sine * sine)];
        const run = 300 * (gradient[0] * direction[0] + gradient[1] * direction[1]);
        const path = document.splines[0];
        path.points = [
          [...start, elevation],
          [250 + 150 * direction[0], 250 + 150 * direction[1], elevation + rise],
        ];
        if (curved) {
          path.curved = true;
          path.points.splice(1, 0, [
            250 - 50 * direction[1],
            250 + 50 * direction[0],
            elevation + rise / 2,
          ]);
        }
        for (const vertex of document.terrain.vertices)
          vertex.position[2] = deck
            ? 0
            : elevation +
              (((vertex.position[0] - start[0]) * gradient[0] +
                (vertex.position[1] - start[1]) * gradient[1]) *
                rise) /
                run;
        if (deck) {
          // Upright actors must not fit beneath subpixel gaps between independently
          // triangulated bent floor and wall faces. The ground remains well below.
          asset.gameplay.volumes[0].movementHeadroom = 20;
          asset.gameplay.surfaces = [
            {
              id: "deck",
              node: "body",
              height: 0,
              navigationRegion: "deck",
              polygon: [
                [-50, -80],
                [50, -80],
                [50, 80],
                [-50, 80],
              ],
            },
          ];
        }
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
        const layers = compiled.descriptor.asset_geometry.motion_data.layers;
        if (deck && enabled)
          assert.deepEqual(
            layers.map((layer) => layer.length),
            [1, 1, 0],
            "Raised deck must compile into one connected region above the terrain",
          );
        // The flat terrain has four corners; the curved deck has a sampled perimeter.
        // Plane ordering can place the sloped deck before or after the terrain.
        const routeLayer = deck
          ? layers.findIndex((layer) => layer[0]?.polygon.points.length > 4)
          : 0;
        assert.ok(routeLayer >= 0, "Compiled curved deck region must exist");
        const curve = splineCurve(path, document.camera);
        const probes = [];
        for (let repeat = 0; repeat < 3; repeat++) {
          for (const [offset, opening] of [
            [-22, true],
            [0, false],
            [22, true],
            [45, false],
          ]) {
            const fraction = (repeat * 100 + 50 + offset) / curve.getLength();
            const centre = curve.getPointAt(fraction);
            const tangent = sceneToGame(document.camera, curve.getTangentAt(fraction).toArray());
            const dx = tangent[0],
              dy = tangent[1] - tangent[2];
            const length = Math.hypot(dx, dy);
            const normal = [(-dy / length) * 20, (dx / length) * 20];
            const [x, y, z] = sceneToGame(document.camera, centre.toArray());
            probes.push({
              layer: routeLayer,
              start: [x - normal[0], y - z - normal[1]],
              end: [x + normal[0], y - z + normal[1]],
              reachable: enabled && opening,
            });
          }
        }
        assert.equal(probes.length, 12);
        const file = `clearance-${deck ? "deck-" : ""}${curved ? "curved-" : ""}${rotation}-${elevation}-${rise}-${enabled}.level.json`;
        await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
        await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
        results.push({
          file,
          map: file,
          rotation,
          curved,
          deck,
          warnings: compiled.warnings,
          movement_probes: probes,
          route_probes: enabled
            ? probes
                .filter((probe) => probe.reachable)
                .flatMap((probe) => [
                  { ...probe, sector: routeLayer, max_length: 45 },
                  {
                    ...probe,
                    start: probe.end,
                    end: probe.start,
                    sector: routeLayer,
                    max_length: 45,
                  },
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
