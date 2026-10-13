// Preserve the permanent display piece alongside independently removable stock.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { sceneToGame } from "../shared/src/geometry.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";

const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const camera = { kind: "oblique-orthographic", elevation_deg: 35 };
const id = "york-market-north-display-tables",
  base = `3d-assets/york/${id}`;
const bytes = await fs.readFile(`library/${base}/asset.json`);
assert.equal(hash(bytes), "31d3dbed5b3dfbadbf10e710d9a4fc591c2946e48e24d9926e448ea423e197ad");
const descriptor = JSON.parse(bytes);
assert.equal(descriptor.gameplay, undefined);
assert.deepEqual(
  descriptor.parts.map((part) => part.node),
  ["building-041", "building-937"],
);
const ownerBytes = await fs.readFile("library/3d-assets/york-market-state-assembly/asset.json");
assert.equal(hash(ownerBytes), "ad2c6e9b5b6d61858dcd551169aeff2a9781da415cfc0c11db323ea688656555");
const owner = JSON.parse(ownerBytes);
const control = owner.gameplay.movementTransitions.find((row) => row.id === "movement-change-9");
const changing = descriptor.parts[1],
  permanent = descriptor.parts[0];
assert.ok(control.initialSight.includes(changing.node));
assert.ok(!control.appliedSight.includes(changing.node));
assert.ok(!control.initialSight.includes(permanent.node));
assert.ok(!control.appliedSight.includes(permanent.node));
assert.ok(!owner.gameplay.movementBlockers.some((row) => row.node === changing.node));
const source = owner.parts.find((part) => part.node === changing.node);
const origin = sceneToGame(camera, descriptor.source_origin_scene);
const ownerOrigin = sceneToGame(camera, owner.source_origin_scene);
assert.equal(source.obstacle_local_game.points.length, changing.obstacle_local_game.points.length);
for (const [i, point] of changing.obstacle_local_game.points.entries())
  for (const [key, axis] of [
    ["x", 0],
    ["y", 1],
    ["z_bottom", 2],
    ["z_top", 2],
  ])
    assert.ok(
      Math.abs(
        point[key] + origin[axis] - source.obstacle_local_game.points[i][key] - ownerOrigin[axis],
      ) < 0.001,
    );
const ground = Math.min(
  ...descriptor.parts.flatMap((part) => part.obstacle_local_game.points.map((p) => p.z_bottom)),
);
const shape = changing.obstacle_local_game;
const center = [0, 1].map(
  (axis) => shape.points.reduce((sum, p) => sum + (axis ? p.y : p.x), 0) / shape.points.length,
);
changing.appearance = { hide: ["remove-display"] };
descriptor.gameplay = {
  version: 1,
  collision: "parts",
  placementGroundHeight: ground,
  surfaces: [],
  doors: [],
  movementSolids: [permanent.node],
  movementBlockers: [
    {
      id: "permanent-display-footprint",
      node: permanent.node,
      polygon: permanent.obstacle_local_game.points.map((p) => [p.x, p.y]),
      height: ground,
      preserveMovementPrecision: true,
    },
  ],
  movementTransitions: [
    {
      id: "remove-display",
      node: changing.node,
      appearances: ["remove-display"],
      active: control.active,
      definitive: control.definitive,
      waypoint: [...center, ground],
      applyPolygon: [],
      noApplyPolygon: [],
      initial: [
        {
          id: "display-footprint",
          node: changing.node,
          polygon: shape.points.map((p) => [p.x, p.y]),
          height: ground,
          preserveMovementPrecision: true,
        },
      ],
      applied: [],
      initialSight: [changing.node],
      appliedSight: [],
    },
  ],
  draft: {
    issues: [
      "Independent display-piece removal draft; full gameplay parity is not certified.",
      "The model contains only two thin raised pieces, with no supporting legs. Placement grounds their lowest underside; author supporting scenery explicitly when elevated placement is desired.",
      "Only the changing display piece disappears. The neighboring permanent piece remains solid and visible; shared market exclusions and remote mission triggers are not inherited.",
      "Rendered appearance, actor-triggered activation and detailed mesh contact remain under review.",
    ],
  },
};
validateAssetGameplay(descriptor.gameplay, descriptor);
const modelSha256 = hash(await fs.readFile(`library/${base}/${descriptor.model}`));
const reference = {
  id,
  descriptor: `${base}/asset.json`,
  descriptor_sha256: hash(bytes),
  model: `${base}/${descriptor.model}`,
  model_sha256: modelSha256,
  model_scene: descriptor.model_scene,
  resources: descriptor.resources,
};
const output = await fs.mkdtemp("work/needs-review/map-compile/york-display-tables-");
await fs.writeFile(
  `${output}/edits.json`,
  JSON.stringify([
    {
      asset: id,
      descriptorSha256: hash(bytes),
      modelSha256,
      gameplay: descriptor.gameplay,
      partAppearances: { [changing.node]: changing.appearance },
    },
  ]),
);
console.log(output);
const results = [];
const save = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      scope: "static-geometry-only-not-gameplay-parity",
      complete,
      results,
    }),
  );
await save(false);
for (const height of [0, 40])
  for (const rotation of [0, 37, 90, 180]) {
    let document = {
      version: 1,
      map: "Display pieces",
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
    const probes = geometry.movement_transitions.map((transition) => {
      assert.equal(transition.initial_sight.length, 1);
      assert.equal(transition.applied_sight?.length ?? 0, 0);
      const s = geometry.sight_obstacles[transition.initial_sight[0]];
      const points = s.points.map((p) => [p.x, p.y - height]);
      const center = [0, 1].map(
        (axis) => points.reduce((sum, p) => sum + p[axis], 0) / points.length,
      );
      const edges = points.map((p, i) =>
        points[(i + 1) % points.length].map((v, axis) => v - p[axis]),
      );
      edges.sort((a, b) => Math.hypot(...b) - Math.hypot(...a));
      const edge = edges[0],
        length = Math.hypot(...edge);
      const normal = [-edge[1] / length, edge[0] / length];
      const radius =
        12 +
        Math.max(
          ...points.map((p) =>
            Math.abs((p[0] - center[0]) * normal[0] + (p[1] - center[1]) * normal[1]),
          ),
        );
      return {
        id: transition.id,
        layer: 0,
        start: center.map((v, axis) => v - normal[axis] * radius),
        end: center.map((v, axis) => v + normal[axis] * radius),
        initial: false,
        applied: true,
      };
    });
    const routeProbes = probes.flatMap((probe) =>
      [false, true].map((reverse) => ({
        start: reverse ? probe.end : probe.start,
        end: reverse ? probe.start : probe.end,
        layer: 0,
        sector: 0,
        applied_transitions: [probe.id],
        max_length: Math.hypot(probe.end[0] - probe.start[0], probe.end[1] - probe.start[1]) + 1,
      })),
    );
    const removed = new Set(geometry.movement_transitions.flatMap((row) => row.initial_sight));
    const retained = geometry.sight_obstacles.filter((s, i) => s.solid && !removed.has(i));
    assert.equal(retained.length, 2, "Both permanent pieces must remain solid");
    for (const transition of geometry.movement_transitions)
      for (const s of retained) {
        const xs = s.points.map((p) => p.x),
          ys = s.points.map((p) => p.y - height);
        const y = ys.reduce((a, b) => a + b) / ys.length;
        probes.push({
          id: transition.id,
          layer: 0,
          start: [Math.min(...xs) - 10, y],
          end: [Math.max(...xs) + 10, y],
          initial: false,
          applied: false,
        });
      }
    const file = `${rotation}-${height}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
    results.push({
      file,
      transition_probes: probes,
      route_probes: routeProbes,
      warnings: compiled.warnings,
    });
    await save(false);
  }
await save(true);
