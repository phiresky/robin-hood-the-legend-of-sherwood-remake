// Assemble a reusable house from its local permanent and removable parts.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { sceneToGame } from "../shared/src/geometry.ts";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";
import { crossesGroundFootprint } from "./select-panel-route.mjs";

const id = process.argv[2] ?? "nottingham-north-dormer-house";
const reviewed = {
  "nottingham-north-dormer-house": {
    descriptorSha256: "89e1928d387b181ec509f289115524617da53dd572d1a014e61fbe471448aa28",
    remainderSha256: "78d5d5df21bc839e91d8fb4ab8eaf0f7878cd618a5c192a3ab68ee9794893431",
    changing: ["building-554", "building-553"],
    entranceInitiallyBlocked: false,
  },
  "nottingham-northwest-timber-house": {
    descriptorSha256: "a2a3843455d62b18660af16bc1ee039c3f716bbb3c7be20716e52baf91e95c47",
    remainderSha256: "493fe0e3b5589c9b21adc0f29b3d05171e5583155dd2ac92bd97b8ed2bf02c8b",
    changing: ["building-551", "building-552"],
    entranceInitiallyBlocked: true,
  },
};
assert.ok(Object.hasOwn(reviewed, id), "Only reviewed house configurations are supported");
const configuration = reviewed[id];
const base = `3d-assets/nottingham/${id}`;
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const readPinned = async (path, expected) => {
  const bytes = await fs.readFile(`library/3d-assets/${path}/asset.json`);
  assert.equal(hash(bytes), expected);
  return JSON.parse(bytes);
};
const { descriptorSha256 } = configuration;
const descriptor = await readPinned(`nottingham/${id}`, descriptorSha256);
assert.equal(descriptor.gameplay, undefined);
const permanent = await readPinned(`${id}-remainder`, configuration.remainderSha256);
const owner = await readPinned(
  "nottingham-north-state-assembly",
  "7bbc19f9cb3204cec7147daadd2d5200c5b1120ba3b9115d5a7a349de67d74a7",
);
assert.deepEqual(permanent.source_origin_scene, descriptor.source_origin_scene);
const staticNodes = new Set(permanent.parts.map((part) => part.node));
const changing = descriptor.parts.filter((part) => !staticNodes.has(part.node));
assert.deepEqual(
  changing.map((part) => part.node),
  configuration.changing,
);
const camera = { kind: "oblique-orthographic", elevation_deg: 35 };
const origin = sceneToGame(camera, descriptor.source_origin_scene);
const sourceControl = owner.gameplay.movementTransitions.find(
  (control) => control.id === "movement-change-10",
);
assert.ok(sourceControl);
for (const part of descriptor.parts) {
  const sourceOwner = staticNodes.has(part.node) ? permanent : owner;
  const source = sourceOwner.parts.find((candidate) => candidate.node === part.node);
  assert.ok(source);
  const sourceOrigin = sceneToGame(camera, sourceOwner.source_origin_scene);
  const shape = part.obstacle_local_game,
    original = source.obstacle_local_game;
  for (const key of ["solid", "opaque", "mouse"]) assert.equal(shape[key], original[key]);
  assert.equal(shape.points.length, original.points.length);
  shape.points.forEach((point, i) => {
    for (const [key, axis] of [
      ["x", 0],
      ["y", 1],
      ["z_bottom", 2],
      ["z_top", 2],
    ])
      assert.ok(
        Math.abs(point[key] + origin[axis] - original.points[i][key] - sourceOrigin[axis]) < 0.001,
      );
  });
}
for (const part of changing) {
  assert.ok(
    sourceControl.initialSight.includes(part.node) &&
      !sourceControl.appliedSight.includes(part.node),
  );
  assert.ok(!owner.gameplay.movementBlockers.some((blocker) => blocker.node === part.node));
  part.appearance = { hide: ["remove-front-props"] };
}
const ground = Math.min(
  ...descriptor.parts.flatMap((part) =>
    part.obstacle_local_game.points.map((point) => point.z_bottom),
  ),
);
const grounded = changing.filter((part) =>
  part.obstacle_local_game.points.every((point) => Math.abs(point.z_bottom - ground) < 0.001),
);
assert.equal(grounded.length, 1);
assert.equal(grounded[0].node, configuration.changing[0]);
assert.ok(changing[1].obstacle_local_game.points.every((point) => point.z_bottom > ground + 40));
const shape = grounded[0].obstacle_local_game;
const center = ["x", "y"].map(
  (axis) => shape.points.reduce((sum, point) => sum + point[axis], 0) / shape.points.length,
);
descriptor.gameplay = structuredClone(permanent.gameplay);
assert.equal(descriptor.gameplay.movementTransitions, undefined);
descriptor.gameplay.placementGroundHeight = ground;
descriptor.gameplay.movementSolids = [...staticNodes];
// Reserve the outdoor leg to the authored threshold in the permanent wall footprint.
descriptor.gameplay.movementClearances ??= [];
for (const interior of descriptor.gameplay.interiors)
  for (const door of interior.doors) {
    const xs = [door.middle[0], door.outside[0]],
      ys = [door.middle[1], door.outside[1]];
    const left = Math.min(...xs) - 10,
      right = Math.max(...xs) + 10;
    const top = Math.min(...ys) - 6,
      bottom = Math.max(...ys) + 6;
    descriptor.gameplay.movementClearances.push({
      id: `${door.id}-threshold-clearance`,
      node: door.node,
      polygon: [
        [left, top],
        [right, top],
        [right, bottom],
        [left, bottom],
      ],
      height: ground,
      preserveMovementPrecision: true,
    });
  }
for (const part of changing)
  descriptor.gameplay.sightOrder[part.node] = owner.gameplay.sightOrder[part.node];
descriptor.gameplay.movementTransitions = [
  {
    id: "remove-front-props",
    appearances: ["remove-front-props"],
    node: grounded[0].node,
    waypoint: [...center, ground],
    active: true,
    definitive: true,
    applyPolygon: [],
    noApplyPolygon: [],
    initial: [
      {
        id: "front-prop-footprint",
        node: grounded[0].node,
        polygon: shape.points.map((point) => [point.x, point.y]),
        height: ground,
        preserveMovementPrecision: true,
      },
    ],
    applied: [],
    initialSight: changing.map((part) => part.node),
    appliedSight: [],
  },
];
descriptor.gameplay.draft.issues.push(
  "Independent front-prop removal preserves the house's permanent gameplay and interior definitions. Ground blocking follows the removable part's own footprint; shared assembly contours and remote triggers are not inherited.",
);
if (configuration.entranceInitiallyBlocked)
  descriptor.gameplay.draft.issues.push(
    "The removable front prop obstructs the entrance initially. Removing this house's own props opens its entrance; other placed copies keep their independent state.",
  );
validateAssetGameplay(descriptor.gameplay, descriptor);
const modelSha256 = hash(await fs.readFile(`library/${base}/model.glb`));
const reference = {
  id,
  descriptor: `${base}/asset.json`,
  descriptor_sha256: descriptorSha256,
  model: `${base}/model.glb`,
  model_sha256: modelSha256,
  model_scene: descriptor.model_scene,
  resources: descriptor.resources,
};
const output = await fs.mkdtemp(`work/needs-review/map-compile/${id}-gameplay-`);
console.log(output);
await fs.writeFile(
  `${output}/edits.json`,
  JSON.stringify([
    {
      asset: id,
      descriptorSha256,
      modelSha256,
      gameplay: descriptor.gameplay,
      partAppearances: Object.fromEntries(changing.map((part) => [part.node, part.appearance])),
    },
  ]),
);
const results = [];
const save = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({ scope: "static-geometry-only-not-gameplay-parity", complete, results }),
  );
await save(false);
for (const height of [0, 40])
  for (const rotation of [0, 37, 90, 180]) {
    let document = {
      version: 1,
      map: `Independent ${id} copies`,
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
    const compiled = compileMap(document, [0, 0, 1000, 1000], new Map([[id, descriptor]]), {
      bestEffort: true,
    });
    const geometry = compiled.descriptor.asset_geometry;
    const buildings = (geometry.buildings ?? []).flatMap((entry) =>
      entry.Building ? [entry.Building] : [],
    );
    assert.equal(buildings.length, 2, "Each copy retains its own interior");
    assert.equal(geometry.movement_transitions.length, 2);
    const approaches = buildings
      .flatMap((building) => building.doors)
      .map((door) => {
        const receiver = geometry.sight_obstacles.find(
          (s) =>
            s.projection_area?.[0] === door.sector_out &&
            s.projection_area?.[1] === door.layer_out &&
            pointInGameplayPolygon(
              door.point_out,
              s.points.map((p) => [p.x, p.y - p.z_top]),
              true,
            ),
        );
        assert.ok(receiver, "Entrance retains a supported approach");
        const plane = heightPlane(
          (
            receiver.projection_plane ?? receiver.points.slice(0, 3).map((p) => [p.x, p.y, p.z_top])
          ).map(([x, y, z]) => [x, y - z, z]),
        );
        const z = planeHeight(plane, door.point_out);
        return {
          point_out: door.point_out,
          layer: door.layer_out,
          sector: door.sector_out,
          source_world: [door.point_out[0], door.point_out[1] + z, z],
          expected_authorized: !configuration.entranceInitiallyBlocked,
        };
      });
    assert.equal(approaches.length, 2);
    const controlled = new Set(
      geometry.movement_transitions.flatMap((control) => control.initial_sight),
    );
    const permanentSight = geometry.sight_obstacles.flatMap((part, index) =>
      part.solid && !controlled.has(index) ? [index] : [],
    );
    assert.equal(permanentSight.length, 2 * staticNodes.size);
    const transitionProbes = geometry.movement_transitions.map((control) => {
      const prop = control.initial_sight
        .map((index) => geometry.sight_obstacles[index])
        .find((part) => part.points.every((point) => Math.abs(point.z_bottom - height) < 0.001));
      assert.ok(prop);
      const xs = prop.points.map((point) => point.x),
        y =
          prop.points.reduce((sum, point) => sum + point.y - point.z_bottom, 0) /
          prop.points.length;
      const start = [Math.min(...xs) - 15, y],
        end = [Math.max(...xs) + 15, y];
      const applied = !permanentSight.some((index) =>
        crossesGroundFootprint(start, end, geometry.sight_obstacles[index], height),
      );
      return { id: control.id, layer: 0, start, end, initial: false, applied };
    });
    const file = `${rotation}-${height}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
    results.push({
      file,
      permanent_sight: permanentSight,
      transition_probes: transitionProbes,
      building_approaches: approaches,
      warnings: compiled.warnings,
    });
    await save(false);
  }
await save(true);
if (configuration.entranceInitiallyBlocked) {
  const stateDirectory = `${output}/entrance-states`;
  await fs.mkdir(stateDirectory);
  const stateResults = [];
  for (const result of results) {
    const geometry = JSON.parse(await fs.readFile(`${output}/${result.file}`)).asset_geometry;
    const controls = geometry.movement_transitions;
    assert.equal(controls.length, result.building_approaches.length);
    await fs.symlink(`../${result.file}`, `${stateDirectory}/${result.file}`);
    for (let bits = 0; bits < 2 ** controls.length; bits++)
      stateResults.push({
        file: result.file,
        applied_transitions: controls
          .filter((_, i) => bits & (1 << i))
          .map((control) => control.id),
        building_approaches: result.building_approaches.map((probe, i) => ({
          ...probe,
          expected_authorized: (bits & (1 << i)) !== 0,
        })),
      });
  }
  await fs.writeFile(
    `${stateDirectory}/diagnostics.json`,
    JSON.stringify({
      scope: "independent-entrance-state-routes-not-actor-trigger-or-render-proof",
      complete: true,
      results: stateResults,
    }),
  );
}
