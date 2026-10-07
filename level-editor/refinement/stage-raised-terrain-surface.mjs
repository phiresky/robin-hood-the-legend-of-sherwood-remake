import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { transformedObstacle } from "../shared/src/level3d.ts";
import { heightPlane } from "../shared/src/gameplay-plane.ts";
import { movementVolumeHeightSlice } from "../shared/src/movement-volume-height-slice.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

// Unpublished authoring experiment: a physical deck owns its walking surface;
// neighbouring buildings provide collision through their independently placed solids.
const [map, assetId, stage] = process.argv.slice(2);
const clipping = createRequire(new URL("../shared/package.json", import.meta.url))(
  "polygon-clipping",
);
assert.ok(map && assetId && stage, "Provide map, raised terrain asset and gameplay stage");
const document = await readStoredMap(`library/scenes/${map}.rhlos-map.json`, "library");
const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
const changed = new Map(edits.map((edit) => [edit.asset, edit]));
const edit = (id) => {
  if (!changed.has(id))
    changed.set(id, {
      asset: id,
      descriptorSha256: index.find((entry) => entry.id === id).descriptor_sha256,
      gameplay: structuredClone(assets.get(id).gameplay),
    });
  return changed.get(id).gameplay;
};
const terrain = edit(assetId);
assert.equal(terrain.surfaces.length, 0, "Review existing terrain surfaces before replacement");
assert.equal(terrain.projectionReceivers.length, 1);
const receiver = terrain.projectionReceivers[0];
const part = assets.get(assetId).parts.find((part) => part.node === receiver.volume);
assert.ok(part && part.node === receiver.node && part.obstacle_local_game.solid);
const shape = part.obstacle_local_game;
assert.ok(shape.points.every((point) => point.z_top === shape.points[0].z_top));
const placed = document.objects.find((object) => object.node === `asset:${assetId}:${part.node}`);
assert.ok(placed);
const world = transformedObstacle(document, placed);
const top = heightPlane(world.points.map((p) => [p.x, p.y, p.z_top]));
const footprint = world.points.map((p) => [p.x, p.y]);
const removed = structuredClone(terrain.movementBlockers ?? []);
terrain.movementBlockers = [];
terrain.projectionReceivers = [];
terrain.surfaces.push({
  id: `${part.node}-physical-deck`,
  navigationRegion: `${part.node}-physical-deck`,
  node: part.node,
  polygon: shape.points.map((p) => [p.x, p.y]),
  height: shape.points.map((p) => p.z_top),
  preserveMovementPrecision: true,
  acceptsNavigationJoins: true,
  // Collision can split the deck into several independent receiving areas.
  // Generate a receiver per area rather than bind one volume to multiple sectors.
  projectionMaterials: { defaultMaterial: shape.default_material, regions: [] },
});
const selectedSolids = [];
for (const [id, descriptor] of assets) {
  if (id === assetId || descriptor.gameplay?.collision !== "parts") continue;
  const current = changed.get(id)?.gameplay ?? descriptor.gameplay;
  // Reviewed movement volumes and state-controlled parts must retain their
  // authored ownership; promoting them to permanent part collision is unsafe.
  if (current.movementSolids !== undefined) continue;
  const controlled = new Set(
    (current.movementTransitions ?? []).flatMap((transition) => [
      ...(transition.initialSight ?? []),
      ...(transition.appliedSight ?? []),
    ]),
  );
  for (const part of descriptor.parts) {
    if (
      part.collision === "none" ||
      part.mission_profile !== undefined ||
      controlled.has(part.node)
    )
      continue;
    const object = document.objects.find((object) => object.node === `asset:${id}:${part.node}`);
    if (!object?.obstacle?.solid) continue;
    const volume = transformedObstacle(document, object);
    const ring = volume.points.map((p) => [p.x, p.y]);
    const bottom = heightPlane(volume.points.slice(0, 3).map((p) => [p.x, p.y, p.z_bottom]));
    const ceiling = heightPlane(volume.points.slice(0, 3).map((p) => [p.x, p.y, p.z_top]));
    const slice = movementVolumeHeightSlice(ring, top, bottom, ceiling);
    if (slice.length < 3 || !clipping.intersection([footprint], [slice], [ring]).length) continue;
    const gameplay = edit(id);
    // Preserve implicit collision when the asset has no authored planar exclusions.
    if (gameplay.movementSolids === undefined && gameplay.movementBlockers === undefined) continue;
    if (gameplay.movementSolids?.includes(part.node)) continue;
    gameplay.movementSolids = [...(gameplay.movementSolids ?? []), part.node];
    selectedSolids.push({ asset: id, node: part.node });
  }
}
for (const [id, { gameplay }] of changed) {
  gameplay.draft ??= { issues: [] };
  gameplay.draft.issues.push(
    "Unpublished physical terrain candidate: solid-derived clearances require mesh, moved-placement and native route review.",
  );
  validateAssetGameplay(gameplay, assets.get(id));
}
const output = await fs.mkdtemp("work/map-compile/raised-terrain-surface-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify([...changed.values()]));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({ map, assetId, stage, removed, receiver, selectedSolids, top }),
);
console.log(
  JSON.stringify({
    output,
    removedBlockers: removed.length,
    selectedSolids: selectedSolids.length,
  }),
);
