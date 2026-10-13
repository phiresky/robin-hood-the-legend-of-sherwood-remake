// Author independent raised/lowered panels from reviewed part geometry.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { sceneToGame } from "../shared/src/geometry.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";

const camera = { kind: "oblique-orthographic", elevation_deg: 35 };
const reviewed = {
  "york-market-freestanding-canvas-screen": {
    descriptor: "f18fbd5bd78a247ec6ed54aa5ce49eefdbe29a8afe1723bbbdf6cea7754e4b6e",
    model: "767cba16aea4ffb31deea408d46f3379f07b727deece329ff72eb603a61d3663",
    parts: ["building-965", "building-977"],
    control: "lower-screen",
  },
  "york-market-stone-house-striped-awning": {
    descriptor: "be808cc0527ea7f78a959ae3eda292656815ce5a8336f674f06dea89836dca3e",
    model: "b34b841fd2d27efe13a7c195205fe15ed23c9426347eb338895eb94fc92d29e7",
    parts: ["building-935", "building-936", "building-956", "building-957", "building-973"],
    control: "lower-awning",
  },
  "york-market-southeast-canopy-stall": {
    descriptor: "d2ab4d94b4a0afb69e852e940127d010429b08e5f37d4cd054f15e8fa57a4f8e",
    model: "76113b9db889a2fb73ad9847b44af44b5a6c92e1aebde434609228f4a1b05a58",
    parts: ["building-948", "building-960", "building-961", "building-970"],
    control: "lower-canopy",
    assemblyExclusions: ["building-970-ground-blocker-62-0-0"],
  },
  "york-market-west-long-canopy-stall": {
    descriptor: "00075650b779162aa91a6d375e5973918b909c286f4ad97be2c99d7856bec943",
    model: "fee08743d2517d7d6135597fc4d7164359815fd90225c332ccdfef951edd8a69",
    parts: ["building-955", "building-958", "building-959", "building-975"],
    control: "lower-canopy",
    assemblyExclusions: ["building-975-ground-blocker-71-0-0"],
  },
};
const id = process.argv[2] ?? "york-market-freestanding-canvas-screen";
const review = reviewed[id];
assert.ok(review, "Supply a reviewed panel asset ID");
const base = `3d-assets/york/${id}`;
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const bytes = await fs.readFile(`library/${base}/asset.json`);
const descriptorSha256 = hash(bytes);
assert.equal(descriptorSha256, review.descriptor);
const modelSha256 = hash(await fs.readFile(`library/${base}/model.glb`));
assert.equal(modelSha256, review.model);
const descriptor = JSON.parse(bytes);
assert.equal(descriptor.gameplay, undefined);
const ownerBytes = await fs.readFile("library/3d-assets/york-market-state-assembly/asset.json");
assert.equal(hash(ownerBytes), "ad2c6e9b5b6d61858dcd551169aeff2a9781da415cfc0c11db323ea688656555");
const owner = JSON.parse(ownerBytes);
const transition = owner.gameplay.movementTransitions.find(
  (control) => control.id === "movement-change-9",
);
assert.ok(transition);
assert.deepEqual(
  descriptor.parts.map((part) => part.node),
  review.parts,
);
const origin = sceneToGame(camera, descriptor.source_origin_scene);
const ownerOrigin = sceneToGame(camera, owner.source_origin_scene);
for (const part of descriptor.parts) {
  const source = owner.parts.find((candidate) => candidate.node === part.node);
  assert.ok(source);
  const shape = part.obstacle_local_game,
    sourceShape = source.obstacle_local_game;
  assert.equal(shape.points.length, sourceShape.points.length);
  for (const key of ["solid", "opaque", "mouse"]) assert.equal(shape[key], sourceShape[key]);
  for (let i = 0; i < shape.points.length; i++)
    for (const [key, axis] of [
      ["x", 0],
      ["y", 1],
      ["z_bottom", 2],
      ["z_top", 2],
    ]) {
      assert.ok(
        Math.abs(
          shape.points[i][key] + origin[axis] - sourceShape.points[i][key] - ownerOrigin[axis],
        ) < 0.001,
        "Part geometry/frame differs from owner",
      );
    }
}
const raised = descriptor.parts.slice(0, -1),
  lowered = descriptor.parts.at(-1);
const assemblyExclusions = owner.gameplay.movementBlockers.filter((blocker) =>
  review.parts.includes(blocker.node),
);
assert.deepEqual(
  assemblyExclusions.map((blocker) => blocker.id),
  review.assemblyExclusions ?? [],
);
assert.ok(
  assemblyExclusions.every((blocker) => blocker.node === lowered.node),
  "A permanent exclusion on a raised piece needs separate authoring",
);
for (const part of raised)
  assert.ok(
    transition.initialSight.includes(part.node) && !transition.appliedSight.includes(part.node),
  );
assert.ok(
  transition.appliedSight.includes(lowered.node) && !transition.initialSight.includes(lowered.node),
);
const ground = Math.min(...lowered.obstacle_local_game.points.map((point) => point.z_bottom));
for (const part of raised)
  assert.ok(part.obstacle_local_game.points.every((point) => point.z_bottom - ground > 40));
assert.ok(
  lowered.obstacle_local_game.points.every((point) => Math.abs(point.z_bottom - ground) < 1e-6),
);
const shape = lowered.obstacle_local_game;
const center = [0, 1].map(
  (axis) =>
    shape.points.reduce((sum, point) => sum + (axis ? point.y : point.x), 0) / shape.points.length,
);
for (const part of raised) part.appearance = { hide: [review.control] };
lowered.appearance = { show: [review.control] };
descriptor.gameplay = {
  version: 1,
  placementGroundHeight: ground,
  collision: "parts",
  surfaces: [],
  doors: [],
  movementBlockers: [],
  movementTransitions: [
    {
      id: review.control,
      appearances: [review.control],
      node: lowered.node,
      waypoint: [...center, ground],
      active: transition.active,
      definitive: transition.definitive,
      applyPolygon: [],
      noApplyPolygon: [],
      initial: [],
      applied: [
        {
          id: "lowered-footprint",
          node: lowered.node,
          polygon: shape.points.map((point) => [point.x, point.y]),
          height: ground,
          preserveMovementPrecision: true,
        },
      ],
      initialSight: raised.map((part) => part.node),
      appliedSight: [lowered.node],
    },
  ],
  draft: {
    issues: [
      "Independent raised/lowered panel draft; full gameplay parity is not certified.",
      "Starts raised with a clear ground footprint. Explicit activation swaps to the lowered blocking geometry; no surrounding market exclusion or remote trigger is inherited.",
      "Rendered state appearance, actor interaction and mesh contact require review.",
      ...(assemblyExclusions.length
        ? [
            "The standalone raised state remains walkable beneath its elevated parts. The full market assembly retains its separate permanent exclusion; it is not inherited by this independent asset.",
          ]
        : []),
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
const output = await fs.mkdtemp("work/needs-review/map-compile/york-canvas-screen-");
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
      map: "Independent canvas screens",
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
    const probes = geometry.movement_transitions.map((control) => {
      assert.equal(control.initial_sight.length, raised.length);
      assert.equal(control.applied_sight.length, 1);
      const shape = geometry.sight_obstacles[control.applied_sight[0]];
      const xs = shape.points.map((point) => point.x),
        ys = shape.points.map((point) => point.y - point.z_bottom);
      const y = ys.reduce((a, b) => a + b) / ys.length;
      return {
        id: control.id,
        layer: 0,
        start: [Math.min(...xs) - 15, y],
        end: [Math.max(...xs) + 15, y],
        initial: true,
        applied: false,
      };
    });
    const file = `${rotation}-${height}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
    const receivers = geometry.sight_obstacles.filter((obstacle) => obstacle.projection_area);
    assert.equal(receivers.length, 1);
    assert.deepEqual(receivers[0].projection_area, [0, 0]);
    const routeProbes = probes.map((probe) => ({
      start: probe.start,
      end: probe.end,
      layer: 0,
      sector: 0,
      max_length: Math.hypot(probe.end[0] - probe.start[0], probe.end[1] - probe.start[1]) + 1,
    }));
    results.push({
      file,
      transition_probes: probes,
      route_probes: routeProbes,
      warnings: compiled.warnings,
    });
    await report(false);
  }
await report(true);
