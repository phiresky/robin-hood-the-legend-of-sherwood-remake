// Author independent raised/lowered panels from reviewed part geometry.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { sceneToGame } from "../shared/src/geometry.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";
import { crossesGroundFootprint, selectPanelRoute } from "./select-panel-route.mjs";

const camera = { kind: "oblique-orthographic", elevation_deg: 35 };
const reviewed = {
  "york-market-red-canopy-stall": {
    descriptor: "fd887d46b049eba6fa284fc26e5ebe0b1a8de0c1cfe270e7e499cada5c289c1e",
    model: "356476586d1d52e48683bb00f7f7410f962865a7b29f84381340ea8b54a19a78",
    parts: [
      "building-032",
      "building-033",
      "building-034",
      "building-950",
      "building-951",
      "building-972",
    ],
    control: "lower-canopy",
    staticOwner: "york-market-red-canopy-stall-remainder",
    staticDescriptor: "683f30745a8c3c7a565ea769da52e48a60f62055b8a409205c6c7afcee806638",
    initialGroundNodes: ["building-950", "building-951"],
    upperSurfaces: 1,
    groundPassage: false,
  },
  "york-market-striped-canopy-pottery-stall": {
    descriptor: "e03895f7e7d88fb6437842a1e26e558056908b31c900bf05c44872cdaf824b50",
    model: "89091d86ed6aac025a7c1617ab8825f0d6b1607c273f1fba09408fde061d3bf2",
    parts: [
      "building-495",
      "building-496",
      "building-497",
      "building-966",
      "building-967",
      "building-978",
    ],
    control: "lower-canopy",
    staticOwner: "york-market-striped-canopy-pottery-stall-remainder",
    staticDescriptor: "f04c154350457294a515a0f0810c5edf2f790fdec91bbf0cc88055f1616d9b50",
    initialGroundNodes: ["building-966"],
    assemblyExclusions: [
      "building-966-ground-blocker-65-0-0",
      "building-978-ground-blocker-66-0-0",
    ],
  },
  "york-market-timber-house-striped-awning": {
    descriptor: "e163fe2e41d3a92672ca4140ce1f6d1ed45760675ead488aee15339e281dfb2b",
    model: "f8bd5a67219440b33aa962eab2cf8c98936593bbe4f56adb0396ef238ad712ef",
    parts: [
      "building-046",
      "building-939",
      "building-952",
      "building-953",
      "building-954",
      "building-974",
    ],
    control: "lower-awning",
    staticOwner: "york-market-timber-house-striped-awning-remainder",
    staticDescriptor: "56b657002f3e8e66756d019118ff2df52c5c7eb9e1301054d3ffc009238bba82",
  },
  "york-market-southeast-green-canopy-stall": {
    descriptor: "8563e8add8ad3a35de3fe0d7d66456df46c4575aebd1e4a765b1c442aa73198d",
    model: "a8ca058bb43788666e27dca2865bd35697445a66c066b99e80ae5136829cb8eb",
    parts: ["building-031", "building-949", "building-962", "building-971"],
    control: "lower-canopy",
    staticOwner: "york-market-southeast-green-canopy-stall-remainder",
    staticDescriptor: "34d77d0d4877386ba2792e4b19116517f23599290cb6665d469814f2e0e592ff",
    assemblyExclusions: [
      "building-971-ground-blocker-63-0-0",
      "building-971-ground-blocker-63-1-0",
    ],
  },
  "york-market-north-cream-canopy-stall": {
    descriptor: "96d6682f24ba8e1ae4425e3ba708fd1342226f7fab0b8a3be6e610d754f470ca",
    model: "79a9dcbd2e7518b93043e91199126c2690b6f5b3109c035cecb56da124120856",
    parts: ["building-045", "building-963", "building-964", "building-976"],
    control: "lower-canopy",
    staticOwner: "york-market-north-cream-canopy-stall-remainder",
    staticDescriptor: "55aed97ba1e225d5c8d1852fb41250fa4640c0af6bf54944af751389f9285211",
  },
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
let staticOwner;
if (review.staticOwner) {
  const staticBytes = await fs.readFile(`library/3d-assets/${review.staticOwner}/asset.json`);
  assert.equal(hash(staticBytes), review.staticDescriptor);
  staticOwner = JSON.parse(staticBytes);
  const gameplay = staticOwner.gameplay;
  assert.ok(
    Object.keys(gameplay).every((key) =>
      [
        "version",
        "collision",
        "surfaces",
        "jumpZones",
        "jumpSegments",
        "sightOrder",
        "movementBlockers",
        "doors",
        "lifts",
        "interiors",
        "draft",
      ].includes(key),
    ),
    "Static owner has additional gameplay requiring explicit migration",
  );
  assert.equal(gameplay.collision, "parts");
  assert.deepEqual(
    staticOwner.source_origin_scene,
    descriptor.source_origin_scene,
    "Permanent contours require matching local frames",
  );
  assert.equal(gameplay.surfaces.length, review.upperSurfaces ?? 0);
  for (const key of ["surfaces", "jumpZones", "jumpSegments"])
    for (const feature of gameplay[key] ?? [])
      assert.ok(
        staticOwner.parts.some((part) => part.node === feature.node),
        "Permanent feature must belong to a retained part",
      );
  for (const key of ["doors", "lifts", "interiors"])
    assert.equal(
      gameplay[key]?.length ?? 0,
      0,
      "Static owner has features requiring explicit migration",
    );
}
const staticNodes = new Set(staticOwner?.parts.map((part) => part.node) ?? []);
assert.ok([...staticNodes].every((node) => review.parts.includes(node)));
const transition = owner.gameplay.movementTransitions.find(
  (control) => control.id === "movement-change-9",
);
assert.ok(transition);
assert.deepEqual(
  descriptor.parts.map((part) => part.node),
  review.parts,
);
const origin = sceneToGame(camera, descriptor.source_origin_scene);
for (const part of descriptor.parts) {
  const sourceOwner = staticNodes.has(part.node) ? staticOwner : owner;
  const ownerOrigin = sceneToGame(camera, sourceOwner.source_origin_scene);
  const source = sourceOwner.parts.find((candidate) => candidate.node === part.node);
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
const raised = descriptor.parts.slice(0, -1).filter((part) => !staticNodes.has(part.node)),
  lowered = descriptor.parts.at(-1);
const initialGroundNodes = new Set(review.initialGroundNodes ?? []);
assert.ok([...initialGroundNodes].every((node) => raised.some((part) => part.node === node)));
const assemblyExclusions = owner.gameplay.movementBlockers.filter((blocker) =>
  review.parts.includes(blocker.node),
);
assert.deepEqual(
  assemblyExclusions.map((blocker) => blocker.id),
  review.assemblyExclusions ?? [],
);
assert.ok(
  assemblyExclusions.every(
    (blocker) => blocker.node === lowered.node || initialGroundNodes.has(blocker.node),
  ),
  "A permanent exclusion on an unreviewed initial piece needs separate authoring",
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
  assert.ok(
    part.obstacle_local_game.points.every((point) =>
      initialGroundNodes.has(part.node)
        ? Math.abs(point.z_bottom - ground) < 1e-6
        : point.z_bottom - ground > 40,
    ),
  );
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
  surfaces: structuredClone(staticOwner?.gameplay.surfaces ?? []),
  ...(staticOwner?.gameplay.jumpZones
    ? { jumpZones: structuredClone(staticOwner.gameplay.jumpZones) }
    : {}),
  ...(staticOwner?.gameplay.jumpSegments
    ? { jumpSegments: structuredClone(staticOwner.gameplay.jumpSegments) }
    : {}),
  doors: [],
  movementBlockers: (staticOwner?.gameplay.movementBlockers ?? []).map(
    ({ movementContour: _movementContour, ...blocker }) => {
      assert.ok(staticNodes.has(blocker.node), "Permanent contour must belong to a retained part");
      return structuredClone(blocker);
    },
  ),
  ...(staticNodes.size
    ? {
        movementSolids: [...staticNodes],
        sightOrder: Object.fromEntries(
          descriptor.parts.map((part) => {
            const sourceOwner = staticNodes.has(part.node) ? staticOwner : owner;
            const order = sourceOwner.gameplay.sightOrder[part.node];
            assert.ok(Number.isFinite(order));
            return [part.node, order];
          }),
        ),
      }
    : {}),
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
      initial: raised
        .filter((part) => initialGroundNodes.has(part.node))
        .map((part) => ({
          id: `${part.node}-initial-footprint`,
          node: part.node,
          polygon: part.obstacle_local_game.points.map((point) => [point.x, point.y]),
          height: ground,
          preserveMovementPrecision: true,
        })),
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
      "Raised panels preserve permanent asset collision. Explicit activation swaps to the lowered blocking geometry; no surrounding market exclusion or remote trigger is inherited.",
      "Rendered state appearance, actor interaction and mesh contact require review.",
      ...(staticOwner?.gameplay.draft?.issues ?? []),
      ...(initialGroundNodes.size
        ? [
            "Initial ground-level pieces block their own footprints until activation replaces them with the lowered geometry.",
          ]
        : []),
      ...(assemblyExclusions.length
        ? [
            "Assembly-only exclusions attached to changing pieces are not inherited. Each state derives blocking from its own ground-level pieces; permanent parts retain their collision in both states.",
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
        descriptor.parts
          .filter((part) => part.appearance)
          .map((part) => [part.node, part.appearance]),
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
    const controlledSight = new Set(
      geometry.movement_transitions.flatMap((control) => [
        ...control.initial_sight,
        ...control.applied_sight,
      ]),
    );
    const permanentSight = geometry.sight_obstacles.flatMap((shape, index) =>
      shape.solid && !controlledSight.has(index) ? [index] : [],
    );
    assert.equal(permanentSight.length, 2 * staticNodes.size);
    const probes = geometry.movement_transitions.flatMap((control) => {
      assert.equal(control.initial_sight.length, raised.length);
      assert.equal(control.applied_sight.length, 1);
      if (review.groundPassage === false) return [];
      const shape = geometry.sight_obstacles[control.applied_sight[0]];
      const passage = selectPanelRoute(
        shape,
        [...permanentSight, ...control.initial_sight].map(
          (index) => geometry.sight_obstacles[index],
        ),
        height,
      );
      return [
        {
          id: control.id,
          layer: 0,
          ...passage,
          initial: true,
          applied: false,
        },
      ];
    });
    const file = `${rotation}-${height}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
    const receivers = geometry.sight_obstacles.filter((obstacle) => obstacle.projection_area);
    assert.ok(
      receivers.some(
        (receiver) => receiver.projection_area[0] === 0 && receiver.projection_area[1] === 0,
      ),
    );
    if (review.upperSurfaces) assert.ok(receivers.length >= 1 + 2 * review.upperSurfaces);
    else assert.equal(receivers.length, 1);
    const routeProbes = probes.map((probe) => ({
      start: probe.start,
      end: probe.end,
      layer: 0,
      sector: 0,
      max_length: Math.hypot(probe.end[0] - probe.start[0], probe.end[1] - probe.start[1]) + 1,
    }));
    for (const control of geometry.movement_transitions)
      for (const index of [...permanentSight, ...control.initial_sight, ...control.applied_sight]) {
        const shape = geometry.sight_obstacles[index];
        if (!shape.points.every((point) => Math.abs(point.z_bottom - height) < 0.001)) continue;
        const xs = shape.points.map((point) => point.x);
        const y =
          shape.points.reduce((sum, point) => sum + point.y - point.z_bottom, 0) /
          shape.points.length;
        const start = [Math.min(...xs) - 15, y],
          end = [Math.max(...xs) + 15, y];
        const appliedObstacles = [
          ...permanentSight,
          ...control.applied_sight,
          ...geometry.movement_transitions
            .filter((other) => other !== control)
            .flatMap((other) => other.initial_sight),
        ];
        const initialObstacles = [
          ...permanentSight,
          ...geometry.movement_transitions.flatMap((other) => other.initial_sight),
        ];
        probes.push({
          id: control.id,
          layer: 0,
          start,
          end,
          initial: initialObstacles.includes(index)
            ? false
            : !initialObstacles.some((obstacle) =>
                crossesGroundFootprint(start, end, geometry.sight_obstacles[obstacle], height),
              ),
          applied: permanentSight.includes(index)
            ? false
            : !appliedObstacles.some((obstacle) =>
                crossesGroundFootprint(start, end, geometry.sight_obstacles[obstacle], height),
              ),
        });
      }
    results.push({
      file,
      transition_probes: probes,
      route_probes: routeProbes,
      permanent_sight: permanentSight,
      warnings: compiled.warnings,
    });
    await report(false);
  }
await report(true);
