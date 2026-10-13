// Independent furniture removal uses only its own grounded collision pieces.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { sceneToGame } from "../shared/src/geometry.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";

const camera = { kind: "oblique-orthographic", elevation_deg: 35 };
const id = "york-market-tavern-outdoor-furniture";
const base = `3d-assets/york/${id}`;
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const bytes = await fs.readFile(`library/${base}/asset.json`);
const descriptorSha256 = hash(bytes);
assert.equal(descriptorSha256, "48c431bee96c0f4168e0a3563b7d0201fa0654494fde1fe70fe3d68a49f432f4");
const modelSha256 = hash(await fs.readFile(`library/${base}/model.glb`));
assert.equal(modelSha256, "62df2cf262d00bb2e4deeb379d2ca089fa1a41ecb33712ea9a2584fbff59c0b8");
const descriptor = JSON.parse(bytes);
assert.equal(descriptor.gameplay, undefined);
const ownerBytes = await fs.readFile("library/3d-assets/york-market-state-assembly/asset.json");
assert.equal(hash(ownerBytes), "ad2c6e9b5b6d61858dcd551169aeff2a9781da415cfc0c11db323ea688656555");
const owner = JSON.parse(ownerBytes);
const control = owner.gameplay.movementTransitions.find(
  (transition) => transition.id === "movement-change-9",
);
assert.ok(control);
assert.deepEqual(
  descriptor.parts.map((part) => part.node),
  ["building-930", "building-931", "building-932", "building-933"],
);
const origin = sceneToGame(camera, descriptor.source_origin_scene);
const ownerOrigin = sceneToGame(camera, owner.source_origin_scene);
for (const part of descriptor.parts) {
  assert.ok(control.initialSight.includes(part.node) && !control.appliedSight.includes(part.node));
  assert.ok(!owner.gameplay.movementBlockers.some((blocker) => blocker.node === part.node));
  const source = owner.parts.find((candidate) => candidate.node === part.node);
  assert.ok(source);
  const shape = part.obstacle_local_game,
    other = source.obstacle_local_game;
  assert.equal(shape.points.length, other.points.length);
  assert.ok(shape.solid && other.solid);
  for (let i = 0; i < shape.points.length; i++)
    for (const [key, axis] of [
      ["x", 0],
      ["y", 1],
      ["z_bottom", 2],
      ["z_top", 2],
    ]) {
      assert.ok(
        Math.abs(shape.points[i][key] + origin[axis] - other.points[i][key] - ownerOrigin[axis]) <
          0.001,
      );
    }
  part.appearance = { hide: ["remove-furniture"] };
}
const ground = Math.min(
  ...descriptor.parts.flatMap((part) =>
    part.obstacle_local_game.points.map((point) => point.z_bottom),
  ),
);
const grounded = descriptor.parts.filter((part) =>
  part.obstacle_local_game.points.every((point) => Math.abs(point.z_bottom - ground) < 0.001),
);
assert.equal(grounded.length, 3);
for (const part of descriptor.parts.filter((part) => !grounded.includes(part)))
  assert.ok(part.obstacle_local_game.points.every((point) => point.z_bottom - ground > 40));
const shape = grounded[0].obstacle_local_game;
const center = [0, 1].map(
  (axis) =>
    shape.points.reduce((sum, point) => sum + (axis ? point.y : point.x), 0) / shape.points.length,
);
descriptor.gameplay = {
  version: 1,
  placementGroundHeight: ground,
  collision: "parts",
  surfaces: [],
  doors: [],
  movementBlockers: [],
  movementTransitions: [
    {
      id: "remove-furniture",
      appearances: ["remove-furniture"],
      node: grounded[0].node,
      waypoint: [...center, ground],
      active: control.active,
      definitive: control.definitive,
      applyPolygon: [],
      noApplyPolygon: [],
      applied: [],
      initial: grounded.map((part) => ({
        id: `${part.node}-footprint`,
        node: part.node,
        polygon: part.obstacle_local_game.points.map((point) => [point.x, point.y]),
        height: ground,
        preserveMovementPrecision: true,
      })),
      initialSight: descriptor.parts.map((part) => part.node),
      appliedSight: [],
    },
  ],
  draft: {
    issues: [
      "Independent removable furniture draft; full gameplay parity is not certified.",
      "Blocking follows this furniture's three grounded parts. Explicit activation removes all four parts and their sight obstacles; shared market exclusions and remote triggers are not inherited.",
      "Rendered state appearance, actor-triggered interaction and detailed mesh contact require review.",
    ],
  },
};
validateAssetGameplay(descriptor.gameplay, descriptor);
const reference = {
  id,
  descriptor: `${base}/asset.json`,
  descriptor_sha256: descriptorSha256,
  model: `${base}/model.glb`,
  model_sha256: modelSha256,
  model_scene: descriptor.model_scene,
  resources: descriptor.resources,
};
const output = await fs.mkdtemp("work/needs-review/map-compile/york-removable-furniture-");
await fs.writeFile(
  `${output}/edits.json`,
  JSON.stringify([
    {
      asset: id,
      descriptorSha256,
      modelSha256,
      gameplay: descriptor.gameplay,
      partAppearances: Object.fromEntries(
        descriptor.parts.map((part) => [part.node, part.appearance]),
      ),
    },
  ]),
);
console.log(output);
const results = [];
const report = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      scope: "static-geometry-only-not-gameplay-parity",
      complete,
      results,
    }),
  );
await report(false);
for (const height of [0, 40])
  for (const rotation of [0, 37, 90, 180]) {
    let document = {
      version: 1,
      map: "Independent removable furniture",
      size: [1000, 1000],
      camera,
      objects: [],
      groups: [],
      sceneAssets: [],
      assetSources: [],
      terrain: createTerrainGrid([0, 0, 1000, 1000], 500, height),
    };
    for (const x of [250.25, 700.25]) {
      const inserted = insertProjectionAsset(document, descriptor, reference, [x, 500.75, height]);
      document = inserted.document;
      document.groups.find((group) => group.id === inserted.selection.id).transform.rot_deg =
        rotation;
    }
    const compiled = compileMap(document, [0, 0, 1000, 1000], new Map([[id, descriptor]]));
    const geometry = compiled.descriptor.asset_geometry;
    assert.equal(geometry.movement_transitions.length, 2);
    const probes = geometry.movement_transitions.flatMap((transition) => {
      assert.equal(transition.initial_sight.length, 4);
      assert.equal(transition.applied_sight?.length ?? 0, 0);
      const shapes = transition.initial_sight
        .map((index) => geometry.sight_obstacles[index])
        .filter((shape) =>
          shape.points.every((point) => Math.abs(point.z_bottom - height) < 0.001),
        );
      assert.equal(shapes.length, 3);
      return shapes.map((shape) => {
        const xs = shape.points.map((point) => point.x),
          ys = shape.points.map((point) => point.y - point.z_bottom);
        const y = ys.reduce((a, b) => a + b) / ys.length;
        return {
          id: transition.id,
          layer: 0,
          start: [Math.min(...xs) - 15, y],
          end: [Math.max(...xs) + 15, y],
          initial: false,
          applied: true,
        };
      });
    });
    const file = `${rotation}-${height}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
    results.push({ file, transition_probes: probes, warnings: compiled.warnings });
    await report(false);
  }
await report(true);
