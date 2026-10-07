// One-time asset authoring: transfer a bridge footprint out of its background
// ground definition. Compilation still reads only the resulting local metadata.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { gameToScene } from "../shared/src/scene.ts";
import { sceneToGame, applyAffineMatrix, signedPolygonArea } from "../shared/src/geometry.ts";
import { fixedPolygonBoolean } from "../shared/src/fixed-polygon-boolean.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { compileMap } from "../app/src/map-compile.ts";

const stage = process.argv[2];
assert.ok(stage, "Provide a staged bridge definition");
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`));
const document = await readStoredMap("library/scenes/croisement03.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
const bridge = assets.get("croisement03-timber-bridge");
const ground = assets.get("croisement03-terrain");
assert.ok(bridge && ground);
const edit = edits.find((edit) => edit.asset === bridge.id);
assert.ok(edit);
bridge.gameplay = structuredClone(edit.gameplay);
const object = document.objects.find(
  (p) => p.node === `asset:${bridge.id}:${bridge.parts[0].node}`,
);
assert.ok(object);
const matrix = partMatrix(document.camera, document, object);
const world = (point) =>
  sceneToGame(document.camera, applyAffineMatrix(matrix, gameToScene(document.camera, ...point)));
const footprints = bridge.gameplay.surfaces.map((surface) => [
  surface.polygon.map(([x, y], i) => {
    const point = world([
      x,
      y,
      typeof surface.height === "number" ? surface.height : surface.height[i],
    ]);
    assert.ok(Math.abs(point[2]) < 1e-7, "Bridge must meet the authored ground plane");
    return point.slice(0, 2);
  }),
]);
const footprint = fixedPolygonBoolean("union", footprints);
assert.equal(footprint.length, 1);
assert.equal(footprint[0].length, 1);
const deck = footprint[0][0].slice(0, -1);
const changed = [];
for (const surface of ground.gameplay.surfaces) {
  if (
    surface.node !== "$root" ||
    !surface.preserveMovementBoundary ||
    (Array.isArray(surface.height) ? surface.height : [surface.height]).some((z) => z !== 0)
  )
    continue;
  const overlap = fixedPolygonBoolean(
    "intersection",
    [[surface.polygon, ...(surface.holes ?? [])]],
    [footprint],
  );
  if (!overlap.some((polygon) => Math.abs(signedPolygonArea(polygon[0])) > 1e-5)) continue;
  assert.equal(surface.holes.length, surface.holeContours.length);
  surface.holes.push(deck);
  surface.holeContours.push(`${ground.id}/${surface.id}/bridge-owned-deck`);
  changed.push(surface.id);
}
assert.equal(changed.length, 1, "Review ambiguous bridge floor ownership");
const sockets = bridge.gameplay.surfaces.flatMap((surface) => surface.navigationJoins);
assert.equal(sockets.length, 2);
const centers = sockets.map(([a, b]) => a.map((n, i) => (n + b[i]) / 2));
const direction = centers[1].map((n, i) => n - centers[0][i]);
const length = Math.hypot(...direction.slice(0, 2));
bridge.gameplay.doors = centers.map((center, i) => {
  const approach = (distance) =>
    center.map((n, axis) =>
      axis === 2 ? n : n + (((i ? 1 : -1) * direction[axis]) / length) * distance,
    );
  return {
    id: `open-end-${i}`,
    node: bridge.parts[0].node,
    polygon: [],
    outside: approach(24),
    inside: approach(-24),
    middle: center,
    type: 0,
    locked: false,
    unlockable: false,
    allowContinuous: true,
  };
});
for (const asset of [ground, bridge]) validateAssetGameplay(asset.gameplay, asset);
const output = await fs.mkdtemp("work/map-compile/bridge-ground-ownership-");
const reference = document.sceneAssets.find((ref) => ref.id === ground.id);
const bytes = await fs.readFile(`library/${reference.descriptor}`);
assert.equal(createHash("sha256").update(bytes).digest("hex"), reference.descriptor_sha256);
await fs.writeFile(
  `${output}/edits.json`,
  JSON.stringify([
    { ...edit, gameplay: bridge.gameplay },
    { asset: ground.id, descriptorSha256: reference.descriptor_sha256, gameplay: ground.gameplay },
  ]),
);
const bounds = document.exportBounds ?? [0, 0, ...document.size];
const compiled = compileMap(document, bounds, assets, { bestEffort: true });
await fs.writeFile(`${output}/croisement03.level.json`, JSON.stringify(compiled.descriptor));
const removed = { ...document, objects: document.objects.filter((o) => o !== object) };
const without = compileMap(removed, bounds, assets, { bestEffort: true });
// Sample the deck interior, not just its endpoints: duplicated background
// ownership can otherwise hide a missing bridge in a successfully loaded map.
const contains = ([x, y], ring) => {
  let result = false;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    const a = ring[i],
      b = ring[j];
    if (a[1] > y !== b[1] > y && x < ((b[0] - a[0]) * (y - a[1])) / (b[1] - a[1]) + a[0])
      result = !result;
  }
  return result;
};
const owners = (descriptor, point) =>
  descriptor.asset_geometry.motion_data.layers.flatMap((layer, l) =>
    layer.flatMap((area, sector) =>
      contains(point, area.polygon.points) &&
      !area.obstacles.some((obstacle) => contains(point, obstacle.polygon.points))
        ? [{ layer: l, sector }]
        : [],
    ),
  );
const ownership = [];
for (const fraction of [0.1, 0.25, 0.5, 0.75, 0.9]) {
  const local = centers[0].map((n, axis) => n + direction[axis] * fraction);
  const [x, y, z] = world(local);
  const point = [x - bounds[0], y - z - bounds[1]];
  const placed = owners(compiled.descriptor, point);
  const absent = owners(without.descriptor, point);
  assert.equal(placed.length, 1, `Deck ownership must be unique at ${fraction}`);
  assert.equal(absent.length, 0, `Removing the bridge must remove its crossing at ${fraction}`);
  ownership.push({ fraction, point, placed, absent });
}
const passages = bridge.gameplay.doors.map((door) => {
  const endpoints = [door.outside, door.inside].map((local) => {
    const [x, y, z] = world(local);
    const point = [x - bounds[0], y - z - bounds[1]];
    const areas = owners(compiled.descriptor, point);
    assert.equal(areas.length, 1, `${door.id} must have unambiguous endpoints`);
    return { point, areas };
  });
  assert.notDeepEqual(
    endpoints[0].areas,
    endpoints[1].areas,
    `${door.id} must connect the background to the deck`,
  );
  return { id: door.id, endpoints };
});
await fs.writeFile(`${output}/without-bridge.level.json`, JSON.stringify(without.descriptor));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify(
    {
      stage,
      changed,
      deck,
      ownership,
      passages,
      doors: bridge.gameplay.doors,
      warnings: compiled.warnings,
      scope: "Unpublished floor ownership and open-passage candidate",
    },
    null,
    2,
  ),
);
await fs.writeFile(
  `${output}/diagnostics.json`,
  JSON.stringify({
    complete: true,
    scope: "static-geometry-only-not-gameplay-parity",
    results: [
      { file: "croisement03.level.json", map: "croisement03" },
      { file: "without-bridge.level.json", map: "without-bridge" },
    ],
  }),
);
console.log(
  JSON.stringify({
    output,
    changed,
    compiledDoors: compiled.descriptor.asset_geometry.doors.length,
  }),
);
