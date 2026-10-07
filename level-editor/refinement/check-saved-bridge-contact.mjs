// Check the saved scene using editor assets only, including its background ground.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { gameToScene } from "../shared/src/scene.ts";
import { sceneToGame, applyAffineMatrix } from "../shared/src/geometry.ts";

const stage = process.argv[2];
assert.ok(stage, "Provide staged bridge gameplay definitions");
const document = await readStoredMap("library/scenes/croisement03.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
for (const edit of JSON.parse(await fs.readFile(`${stage}/edits.json`))) {
  const asset = assets.get(edit.asset);
  if (asset) asset.gameplay = edit.gameplay;
}
const id = "croisement03-timber-bridge",
  asset = assets.get(id);
const object = document.objects.find((p) => p.node === `asset:${id}:${asset.parts[0].node}`);
assert.ok(object);
const bounds = document.exportBounds ?? [0, 0, ...document.size];
const compiled = compileMap(document, bounds, assets, { bestEffort: true });
const geometry = compiled.descriptor.asset_geometry;
const matrix = partMatrix(document.camera, document, object);
const ends = asset.gameplay.surfaces
  .flatMap((s) => s.navigationJoins)
  .map(([a, b]) => a.map((n, i) => (n + b[i]) / 2));
assert.equal(ends.length, 2);
const delta = ends[1].map((n, i) => n - ends[0][i]),
  length = Math.hypot(...delta.slice(0, 2));
const inside = (point, ring) => {
  let result = false;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    const a = ring[i],
      b = ring[j];
    if (
      a[1] > point[1] !== b[1] > point[1] &&
      point[0] < ((b[0] - a[0]) * (point[1] - a[1])) / (b[1] - a[1]) + a[0]
    )
      result = !result;
  }
  return result;
};
const contacts = [];
for (const [end, center] of ends.entries())
  for (const offset of [-15, 15, 40]) {
    const local = center.map((n, axis) =>
      axis === 2 ? n : n + (((end ? 1 : -1) * delta[axis]) / length) * offset,
    );
    const [x, y, z] = sceneToGame(
      document.camera,
      applyAffineMatrix(matrix, gameToScene(document.camera, ...local)),
    );
    const point = [x - bounds[0], y - z - bounds[1]];
    const areas = geometry.motion_data.layers.flatMap((layer, l) =>
      layer.flatMap((area, sector) =>
        inside(point, area.polygon.points) &&
        !area.obstacles.some((o) => inside(point, o.polygon.points))
          ? [{ layer: l, sector }]
          : [],
      ),
    );
    const receivers = geometry.sight_obstacles.flatMap((receiver, index) =>
      Array.isArray(receiver.projection_area) &&
      inside(
        point,
        receiver.points.map((p) => [p.x, p.y - p.z_top]),
      )
        ? [{ index, area: receiver.projection_area, material: receiver.default_material }]
        : [],
    );
    contacts.push({ end, offset, point, height: z, areas, receivers });
  }
const outside = contacts.filter((c) => c.offset === 40);
assert.ok(
  outside.every((c) => c.areas.length === 1),
  "External probes must identify a unique region",
);
assert.deepEqual(
  outside[0].areas,
  outside[1].areas,
  "External bridge banks must share the tested region",
);
const { layer, sector } = outside[0].areas[0];
const output = await fs.mkdtemp("work/map-compile/saved-bridge-contact-");
const file = "croisement03.level.json";
await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
await fs.writeFile(`${output}/contact-report.json`, JSON.stringify({ stage, contacts }, null, 2));
await fs.writeFile(
  `${output}/diagnostics.json`,
  JSON.stringify({
    complete: true,
    scope: "static-geometry-only-not-gameplay-parity",
    results: [
      {
        file,
        map: "croisement03",
        layer,
        sector,
        routes: [outside.map((c) => c.point)],
        same_receiver_routes: outside.every((c) => c.receivers.length === 0),
      },
    ],
  }),
);
console.log(JSON.stringify({ output, contacts }));
