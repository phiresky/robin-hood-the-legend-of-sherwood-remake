import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readStoredMap } from "../pipeline/src/stored-map.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
const [stage] = process.argv.slice(2);
assert.ok(stage);
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
assert.equal(edits[0].asset, "york-precinct-east-wall-stair");
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
const entry = index.find((entry) => entry.id === "york-cathedral-precinct-raised-terrain");
const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`, "utf8");
assert.equal(createHash("sha256").update(bytes).digest("hex"), entry.descriptor_sha256);
const descriptor = JSON.parse(bytes);
const gameplay = structuredClone(descriptor.gameplay);
const surface = gameplay.surfaces.find((surface) => surface.id === "building-087-walk-0");
assert.ok(surface);
const document = await readStoredMap("library/scenes/york.rhlos-map.json", "library");
const stairPlacement = document.groups.find((group) => group.id === edits[0].asset).transform;
const receiverPlacement = document.groups.find((group) => group.id === entry.id).transform;
assert.equal(stairPlacement.rot_deg, 0);
assert.equal(receiverPlacement.rot_deg, 0);
const floor = edits[0].gameplay.surfaces.find((surface) => surface.id === "building-868-walk-0");
const plane = heightPlane(
  floor.polygon.map(([x, y], i) => [
    x + stairPlacement.dx - receiverPlacement.dx,
    y + stairPlacement.dy - receiverPlacement.dy,
    floor.height[i] + stairPlacement.dz - receiverPlacement.dz,
  ]),
);
const changes = [];
for (const [x, y] of [
  [517, -272.948115040608],
  [499, -252.948115040608],
]) {
  const index = surface.polygon.findIndex((point) => Math.hypot(point[0] - x, point[1] - y) < 1e-6);
  assert.ok(index >= 0);
  const before = surface.polygon[index];
  const height = Array.isArray(surface.height) ? surface.height[index] : surface.height;
  const t = (height - planeHeight(plane, before)) / (plane[0] ** 2 + plane[1] ** 2);
  const distance = Math.abs(t) * Math.hypot(plane[0], plane[1]);
  assert.ok(distance < 3, "Receiver contact exceeds authoring bound");
  surface.polygon[index] = before.map((value, axis) => value + t * plane[axis]);
  changes.push({ index, before, after: surface.polygon[index], distance });
}
surface.preserveMovementPrecision = true;
edits.push({ asset: entry.id, descriptorSha256: entry.descriptor_sha256, gameplay });
const output = await fs.mkdtemp("work/map-compile/precinct-stair-receiver-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({
    stage,
    surface: surface.id,
    changes,
    scope: "Candidate receiver precision; native traversal required before publication",
  }),
);
console.log(JSON.stringify({ output }));
