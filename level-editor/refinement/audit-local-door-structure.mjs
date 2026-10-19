// Propose asset-local structural references for review; never edit the library.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { createRequire } from "node:module";
import { prepareCorridorStates } from "../shared/src/compile-navigation-graph.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { movementVolumeHeightSlice } from "../shared/src/movement-volume-height-slice.ts";
import { fixedPolygonBoolean } from "../shared/src/fixed-polygon-boolean.ts";

const { default: earcut, flatten } = createRequire(
  new URL("../shared/package.json", import.meta.url),
)("earcut");

const [directory, gameplayStage] = process.argv.slice(2);
assert.ok(directory, "Supply a saved-map export directory with building-owners.json");
const owners = JSON.parse(await fs.readFile(`${directory}/building-owners.json`));
assert.equal(owners.complete, true);
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const staged = new Map(
  (gameplayStage ? JSON.parse(await fs.readFile(`${gameplayStage}/edits.json`)) : []).map(
    (edit) => [edit.asset, edit],
  ),
);
const scenes = new Map(),
  descriptors = new Map();
const results = [];
for (const owner of owners.results.filter((row) => row.blocked)) {
  if (!scenes.has(owner.map))
    scenes.set(
      owner.map,
      JSON.parse(await fs.readFile(`library/scenes/${owner.map}.rhlos-map.json`)),
    );
  const scene = scenes.get(owner.map);
  const reference = [...(scene.assetSources ?? []), ...(scene.sceneAssets ?? [])].find(
    (r) => r.id === owner.asset,
  );
  assert.ok(reference, `Missing pinned asset ${owner.asset}`);
  if (!descriptors.has(reference.descriptor)) {
    const bytes = await fs.readFile(`library/${reference.descriptor}`);
    assert.equal(hash(bytes), reference.descriptor_sha256);
    const descriptor = JSON.parse(bytes);
    const edit = staged.get(descriptor.id);
    if (edit) {
      assert.equal(edit.descriptorSha256, reference.descriptor_sha256);
      if (edit.modelSha256 !== undefined) assert.equal(edit.modelSha256, reference.model_sha256);
      descriptor.gameplay = edit.gameplay;
    }
    descriptors.set(reference.descriptor, descriptor);
  }
  const descriptor = descriptors.get(reference.descriptor),
    gameplay = descriptor.gameplay;
  const door = gameplay.interiors
    .find((room) => room.id === owner.room)
    ?.doors.find((d) => d.id === owner.door);
  assert.ok(door, `Missing local door ${owner.asset}/${owner.door}`);
  const floor = [0, 0, door.outside[2]];
  const project = ([x, y]) => [x, y - floor[2]];
  const intersects = (polygons) => {
    const obstacles = polygons.flatMap((polygon) => {
      const { vertices, holes, dimensions } = flatten(polygon.map((ring) => ring.map(project)));
      const indices = earcut(vertices, holes, dimensions);
      return Array.from({ length: indices.length / 3 }, (_, triangle) => ({
        state_id: 0,
        polygon: {
          points: [0, 1, 2].map((corner) => {
            const index = indices[triangle * 3 + corner] * dimensions;
            return [vertices[index], vertices[index + 1]];
          }),
        },
      }));
    });
    return !prepareCorridorStates(obstacles)(project(door.outside), project(door.middle)).length;
  };
  const blockers = (gameplay.movementBlockers ?? [])
    .filter((blocker) => {
      const heights =
        typeof blocker.height === "number"
          ? blocker.polygon.map(() => blocker.height)
          : blocker.height;
      const plane = heightPlane(blocker.polygon.map(([x, y], i) => [x, y, heights[i]]));
      if (
        [door.outside, door.middle].some(
          (point) => Math.abs(planeHeight(plane, point) - floor[2]) > 0.001,
        )
      )
        return false;
      return intersects([[blocker.polygon, ...(blocker.holes ?? [])]]);
    })
    .map((blocker) => blocker.id);
  const movementSolid = (id) =>
    gameplay.movementSolids?.includes(id) ?? gameplay.movementBlockers === undefined;
  const shapes = [
    ...(gameplay.collision === "parts"
      ? descriptor.parts
          .filter((part) => part.collision !== "none" && part.mission_profile === undefined)
          .map((part) => ({ id: part.node, shape: part.obstacle_local_game }))
      : []),
    ...(gameplay.volumes ?? []),
  ];
  const volumes = shapes
    .filter(({ id, shape, movementHeadroom = 0 }) => {
      if (!shape?.solid || !movementSolid(id)) return false;
      const footprint = shape.points.map((p) => [p.x, p.y]);
      const bottom = heightPlane(
        shape.points.map((p) => [p.x, p.y, p.z_bottom - movementHeadroom]),
        false,
      );
      const top = heightPlane(
        shape.points.map((p) => [p.x, p.y, p.z_top]),
        false,
      );
      const slice = movementVolumeHeightSlice(footprint, floor, bottom, top);
      if (slice.length < 3) return false;
      let regions = fixedPolygonBoolean("intersection", [footprint], [[slice]]);
      for (const clearance of gameplay.movementClearances ?? []) {
        const heights =
          typeof clearance.height === "number"
            ? clearance.polygon.map(() => clearance.height)
            : clearance.height;
        if (heights.some((height) => Math.abs(height - floor[2]) > 0.001)) continue;
        regions = fixedPolygonBoolean("difference", regions, [
          [clearance.polygon, ...(clearance.holes ?? [])],
        ]);
      }
      return intersects(regions);
    })
    .map(({ id }) => id);
  results.push({
    ...owner,
    descriptor: reference.descriptor,
    descriptorSha256: reference.descriptor_sha256,
    model: reference.model,
    modelSha256: reference.model_sha256,
    gameplaySha256: hash(JSON.stringify(gameplay)),
    proposed: { volumes, blockers },
  });
}
const output = await fs.mkdtemp("work/needs-review/map-compile/local-door-structure-");
await fs.writeFile(
  `${output}/proposals.json`,
  JSON.stringify(
    {
      scope: "unreviewed-local-collision-candidates-not-traversal-proof",
      limitations: [
        "Uses unrounded asset-local anchors and horizontal floor slices; rotated or quantized placements can add obstructions.",
        "Does not identify barriers owned by other placed assets; candidates require native and browser checks before publication.",
      ],
      source: directory,
      ...(gameplayStage ? { gameplayStage } : {}),
      results,
    },
    null,
    2,
  ),
);
console.log(output);
console.log(
  `${results.length} doors; ${results.filter((r) => r.proposed.volumes.length || r.proposed.blockers.length).length} have local structural candidates`,
);
