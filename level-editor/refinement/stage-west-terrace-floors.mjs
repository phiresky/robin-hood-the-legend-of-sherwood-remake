// Stage the terrace's lower deck and ramp as physical asset-local walking floors.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const [stage] = process.argv.slice(2);
assert.ok(stage, "Supply the plateau gameplay stage");
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`));
const asset = "lincoln-west-tower-terrace";
const root = `library/3d-assets/lincoln/${asset}`;
const bytes = await fs.readFile(`${root}/asset.json`);
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const descriptor = JSON.parse(bytes);
const edit = edits.find((edit) => edit.asset === asset);
assert.ok(edit);
assert.equal(edit.descriptorSha256, hash(bytes));
const modelSha256 = hash(await fs.readFile(`${root}/${descriptor.model}`));
if (edit.modelSha256) assert.equal(edit.modelSha256, modelSha256);
edit.modelSha256 = modelSha256;
const gameplay = edit.gameplay;
assert.equal(gameplay.surfaces.length, 0);
const nodes = ["building-060", "building-069", "building-409"];
assert.equal(gameplay.projectionReceivers.length, nodes.length);
assert.ok(gameplay.projectionReceivers.every((receiver) => nodes.includes(receiver.node)));
const changes = [];
for (const node of nodes) {
  const shape = descriptor.parts.find((part) => part.node === node)?.obstacle_local_game;
  assert.ok(shape?.solid && !shape.material_indices.length);
  const vertices = shape.points.map((p) => [p.x, p.y, p.z_top]);
  const plane = heightPlane(vertices);
  assert.ok(vertices.every((p) => Math.abs(planeHeight(plane, p) - p[2]) < 1e-4));
  gameplay.surfaces.push({
    id: `${node}-physical-floor`,
    node,
    navigationRegion: "lower-terrace",
    polygon: vertices.map(([x, y]) => [x, y]),
    height: vertices.map(([x, y]) => planeHeight(plane, [x, y])),
    preserveMovementPrecision: true,
    projectionMaterials: { defaultMaterial: shape.default_material, regions: [] },
  });
  gameplay.movementSolids ??= [];
  if (!gameplay.movementSolids.includes(node)) gameplay.movementSolids.push(node);
  changes.push({ node, plane });
}
gameplay.projectionReceivers = [];
// Keep the lower deck's walls and both gate states on its physical floor.
// The overhead cover has an elevated underside and does not fill the passage.
const gate = gameplay.movementTransitions.find((transition) => transition.id === "door-change-2");
assert.ok(gate && !gate.initial.length && !gate.applied.length);
const controlled = new Set([...gate.initialSight, ...gate.appliedSight]);
for (const part of descriptor.parts) {
  if (!controlled.has(part.node) && !gameplay.movementSolids.includes(part.node))
    gameplay.movementSolids.push(part.node);
}
for (const [state, sight] of [
  ["initial", gate.initialSight],
  ["applied", gate.appliedSight],
]) {
  for (const node of sight) {
    const shape = descriptor.parts.find((part) => part.node === node).obstacle_local_game;
    assert.ok(shape.points.every((point) => point.z_bottom < 147.001 && point.z_top > 147.001));
    gate[state].push({
      id: `${node}-lower-deck-barrier`,
      node,
      polygon: shape.points.map((point) => [point.x, point.y]),
      height: 147.001,
      preserveMovementPrecision: true,
    });
  }
}
gameplay.draft.issues.push(
  "Unpublished lower terrace floors: verify visible support, passages, reveal states and moved neighboring tower approaches.",
);
validateAssetGameplay(gameplay, descriptor);
const output = await fs.mkdtemp("work/needs-review/map-compile/west-terrace-floors-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(`${output}/review.json`, JSON.stringify({ stage, changes }));
console.log(output);
