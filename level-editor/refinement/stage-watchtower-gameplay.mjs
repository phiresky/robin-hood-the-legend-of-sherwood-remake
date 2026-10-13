// Recover an independent tower from its matching asset-local assembly definition.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { gameplayOwnerDependencies } from "./gameplay-owner-dependencies.mjs";

const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json"));
const read = async (id) => {
  const entry = index.assets.find((entry) => entry.id === id);
  assert.ok(entry);
  const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
  assert.equal(hash(bytes), entry.descriptor_sha256);
  const modelSha256 = hash(await fs.readFile(`library/3d-assets/${entry.model}`));
  if (entry.model_sha256 !== undefined) assert.equal(modelSha256, entry.model_sha256);
  return { entry: { ...entry, model_sha256: modelSha256 }, descriptor: JSON.parse(bytes) };
};
const target = await read("nottingham-castle-watchtower");
const owner = await read("nottingham-castle-hall-and-watchtower");
assert.ok(
  !target.descriptor.gameplay,
  "Tower already has definitions; review before replacing them",
);
assert.ok(
  target.descriptor.source_origin_scene.every(
    (value, axis) => Math.abs(value - owner.descriptor.source_origin_scene[axis]) < 1e-8,
  ),
);
const nodes = new Set(target.descriptor.parts.map((part) => part.node));
for (const part of target.descriptor.parts) {
  const source = owner.descriptor.parts.find((candidate) => candidate.node === part.node);
  assert.deepEqual(part.obstacle_local_game, source?.obstacle_local_game);
  assert.equal(part.default_hidden, source.default_hidden);
  assert.deepEqual(gameplayOwnerDependencies(owner.descriptor, part.node).movementTransitions, []);
}
const source = owner.descriptor.gameplay;
const owned = (rows = []) => structuredClone(rows.filter((row) => nodes.has(row.node)));
const surfaces = owned(source.surfaces);
assert.equal(surfaces.length, 1);
assert.equal(surfaces[0].id, "building-537-walk-0");
assert.deepEqual(surfaces[0].projectionMaterials.regions, []);
const roof = surfaces[0];
const plane = heightPlane(roof.projectionMaterials.planePoints);
const lights = owned(source.lights).filter((light) =>
  light.receivers?.some(
    (point) =>
      Math.abs(point[2] - planeHeight(plane, point)) < 1e-5 &&
      pointInGameplayPolygon(point, roof.polygon, true),
  ),
);
const entrances = source.interiors
  .flatMap((room) => room.doors)
  .filter(
    (door) =>
      Math.abs(door.outside[2] - planeHeight(plane, door.outside)) < 1e-5 &&
      pointInGameplayPolygon(door.outside, roof.polygon, true),
  );
assert.equal(entrances.length, 1, "Expected one doorway supported by the tower's own floor");
const wall = target.descriptor.parts.find((part) => part.node === "building-536");
assert.ok(wall);
const wallBoundary = wall.obstacle_local_game.points.map((point) => [point.x, point.y]);
assert.ok(!nodes.has(entrances[0].node), "Review newly tower-owned entrance geometry");
assert.ok(
  !pointInGameplayPolygon(entrances[0].inside, wallBoundary, true),
  "The hall-owned entrance must not silently become a tower room",
);
const gameplay = {
  version: 1,
  collision: "parts",
  surfaces,
  doors: [],
  sightOrder: Object.fromEntries(
    Object.entries(source.sightOrder).filter(([node]) => nodes.has(node)),
  ),
  movementSolids: source.movementSolids.filter((node) => nodes.has(node)),
  movementClearances: owned(source.movementClearances),
  lights,
  interiors: [],
  draft: {
    issues: [
      "Standalone tower gameplay restored from matching asset-local assembly parts; complete parity is not certified.",
      "The nearby upper doorway belongs to the hall shaft, not this tower. Supply roof access with surrounding assets; no interior or ground-level entrance is authored on the standalone tower.",
      "Assembly lighting aimed at hall-only receivers is not assigned to the standalone tower; independent lighting ownership remains under review.",
      "Movement clearances, external receiver bindings and roof visual contact require moved-asset review.",
    ],
  },
};
validateAssetGameplay(gameplay, target.descriptor);
const output = await fs.mkdtemp("work/needs-review/map-compile/watchtower-gameplay-");
await fs.writeFile(
  `${output}/edits.json`,
  JSON.stringify([
    {
      asset: target.entry.id,
      descriptorSha256: target.entry.descriptor_sha256,
      modelSha256: target.entry.model_sha256,
      gameplay,
    },
  ]),
);
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({
    scope: "unpublished-independent-tower-definition-requires-placement-and-door-review",
    target: target.entry.id,
    owner: owner.entry.id,
    ownerDescriptorSha256: owner.entry.descriptor_sha256,
    ownerModelSha256: owner.entry.model_sha256,
    matchedNodes: [...nodes],
    excludedHallDoor: entrances[0],
    excludedAssemblyLights: owned(source.lights).filter(
      (light) => !lights.some((kept) => kept.id === light.id),
    ),
  }),
);
console.log(output);
