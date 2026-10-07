import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

// Explicit authoring experiment: promote selected receiver tops to physical floors.
// A receiver is not necessarily walkable, so callers must select reviewed parts.
const [map, stage, ...selections] = process.argv.slice(2);
assert.ok(map && stage && selections.length, "Provide map, stage and asset/receiver selections");
const document = await readStoredMap(`library/scenes/${map}.rhlos-map.json`, "library");
const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
const pins = new Map(document.assetSources.map((source) => [source.id, source.descriptor_sha256]));
for (const edit of edits) assert.equal(edit.descriptorSha256, pins.get(edit.asset), edit.asset);
const changes = [];
for (const selection of selections) {
  const [id, receiverId, extra] = selection.split("/");
  assert.ok(id && receiverId && !extra, `Invalid selection: ${selection}`);
  const descriptor = assets.get(id);
  assert.ok(descriptor?.gameplay, `Missing gameplay: ${id}`);
  let edit = edits.find((edit) => edit.asset === id);
  if (!edit) {
    edit = {
      asset: id,
      descriptorSha256: pins.get(id),
      gameplay: structuredClone(descriptor.gameplay),
    };
    edits.push(edit);
  }
  const gameplay = edit.gameplay;
  const receiver = gameplay.projectionReceivers?.find((receiver) => receiver.id === receiverId);
  assert.ok(receiver, `Missing receiver: ${selection}`);
  const part = descriptor.parts.find((part) => part.node === receiver.volume);
  assert.ok(
    part && part.node === receiver.node,
    `Receiver needs a direct part volume: ${selection}`,
  );
  const shape = part.obstacle_local_game;
  assert.ok(shape?.solid, `Receiver has no solid support: ${selection}`);
  assert.ok(!shape.material_indices?.length, `Review receiver material partitions: ${selection}`);
  assert.ok(
    gameplay.movementSolids?.includes(part.node) ||
      (gameplay.movementSolids === undefined && gameplay.movementBlockers === undefined),
    `Receiver support is not selected for movement collision: ${selection}`,
  );
  const authoredVertices = shape.points.map((point) => [point.x, point.y, point.z_top]);
  const plane = heightPlane(authoredVertices);
  // Preserve the volume's fitted top plane rather than carry its rounded fourth
  // height into a new floor. Near edge-on rotations amplify that small residual.
  const vertices = authoredVertices.map(([x, y]) => [x, y, planeHeight(plane, [x, y])]);
  const surface = {
    id: `${receiver.id}-physical-floor`,
    navigationRegion: `${receiver.id}-physical-floor`,
    node: receiver.node,
    polygon: vertices.map(([x, y]) => [x, y]),
    height: vertices.map(([, , z]) => z),
    preserveMovementPrecision: true,
    navigationJoins: vertices.map((point, index) => [
      point,
      vertices[(index + 1) % vertices.length],
    ]),
    projectionMaterials: { defaultMaterial: shape.default_material, regions: [] },
  };
  assert.ok(!gameplay.surfaces.some((existing) => existing.id === surface.id));
  gameplay.surfaces.push(surface);
  gameplay.projectionReceivers = gameplay.projectionReceivers.filter((item) => item !== receiver);
  gameplay.draft ??= { issues: [] };
  gameplay.draft.issues.push(
    "Unpublished receiver-floor candidate: mesh coverage, door approaches and moved terrain contacts require review.",
  );
  validateAssetGameplay(gameplay, descriptor);
  changes.push({
    asset: id,
    receiver,
    surface,
    plane,
    maximumHeightCorrection: Math.max(
      ...authoredVertices.map((point, index) => Math.abs(point[2] - vertices[index][2])),
    ),
  });
}
const output = await fs.mkdtemp("work/map-compile/receiver-floors-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(`${output}/review.json`, JSON.stringify({ map, stage, changes }));
console.log(JSON.stringify({ output, floors: changes.length }));
