// Diagnose a discarded turret floor using its retained asset-local receiver.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const scene = JSON.parse(await fs.readFile("library/scenes/lincoln.rhlos-map.json"));
const edits = [];
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
for (const asset of ["lincoln-keep-annex-round-turret", "lincoln-keep-annex"]) {
  const source = scene.assetSources.find((entry) => entry.id === asset);
  assert.ok(source);
  const bytes = await fs.readFile(`library/${source.descriptor}`);
  assert.equal(hash(bytes), source.descriptor_sha256);
  assert.equal(hash(await fs.readFile(`library/${source.model}`)), source.model_sha256);
  const descriptor = JSON.parse(bytes);
  const gameplay = structuredClone(descriptor.gameplay);
  if (asset.endsWith("round-turret")) {
    assert.equal(gameplay.surfaces.length, 0);
    const part = descriptor.parts.find((part) => part.node === "building-228");
    assert.ok(part?.obstacle_local_game?.projection_area);
    const points = part.obstacle_local_game.points;
    assert.ok(points.every((point) => Math.abs(point.z_top - 472.00104) < 1e-6));
    gameplay.surfaces.push({
      id: "building-228-walk-0",
      node: part.node,
      navigationRegion: "round-turret-floor",
      polygon: points.map((point) => [point.x, point.y]),
      height: points.map((point) => point.z_top),
      holes: [],
      preserveMovementPrecision: true,
    });
    gameplay.draft.issues.push(
      "Diagnostic receiver restoration only: retained floor height is 472.00104, but visible turret slabs overlap it at 476/484/486. Reconcile mesh geometry before publication.",
    );
  } else {
    const door = gameplay.doors.find((door) => door.id === "passage-1/door-0");
    assert.deepEqual(door.outsideAnchor, [-27, 61.88272614076818, 472.00104]);
    delete door.outsideAnchor;
    const transition = gameplay.movementTransitions.find((entry) => entry.id === "door-change-7");
    assert.deepEqual(transition.waypointAnchor, [-27, 61.88272614076818, 472.00104]);
    delete transition.waypointAnchor;
    gameplay.draft.issues.push(
      "Diagnostic gate binding: select the actual outside endpoint on the separate turret; verify control binding and moved assemblies before publication.",
    );
  }
  validateAssetGameplay(gameplay, descriptor);
  edits.push({
    asset,
    descriptorSha256: source.descriptor_sha256,
    modelSha256: source.model_sha256,
    gameplay,
  });
}
const output = await fs.mkdtemp("work/needs-review/map-compile/annex-turret-floor-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
console.log(output);
