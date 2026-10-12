// Unpublished authoring experiment: lift flat click contours onto their local floor.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const [stage] = process.argv.slice(2);
assert.ok(stage);
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`));
const changes = [];
for (const [id, zoneId, surfaceId] of [
  ["nottingham-north-street-roof-edge-slab", "jump-zone-34", "building-140-walk-0"],
  ["nottingham-upper-green-house", "jump-zone-33", "building-159-walk-0"],
]) {
  const root = `library/3d-assets/nottingham/${id}`;
  const bytes = await fs.readFile(`${root}/asset.json`);
  const descriptor = JSON.parse(bytes);
  const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
  let edit = edits.find((entry) => entry.asset === id);
  if (edit) assert.equal(edit.descriptorSha256, hash(bytes));
  else {
    edit = {
      asset: id,
      descriptorSha256: hash(bytes),
      modelSha256: hash(await fs.readFile(`${root}/${descriptor.model}`)),
      gameplay: descriptor.gameplay,
    };
    edits.push(edit);
  }
  const zone = edit.gameplay.jumpZones.find((entry) => entry.id === zoneId);
  const surface = edit.gameplay.surfaces.find((entry) => entry.id === surfaceId);
  assert.ok(zone && surface);
  const points = surface.polygon.map(([x, y], i) => {
    const z = Array.isArray(surface.height) ? surface.height[i] : surface.height;
    return [x, y - z, z];
  });
  const plane = heightPlane(points);
  const before = structuredClone(zone);
  zone.polygon = zone.polygon.map(([x, y, z]) => {
    const projectedY = y - z;
    const height = planeHeight(plane, [x, projectedY]);
    return [x, projectedY + height, height];
  });
  zone.node = surface.node;
  validateAssetGameplay(edit.gameplay, descriptor);
  changes.push({ id, before, after: zone, surface: surface.id });
}
const output = await fs.mkdtemp("work/map-compile/roof-jump-zone-planes-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({
    stage,
    changes,
    status: "unpublished: requires rotated click and floor coverage checks",
  }),
);
console.log(output);
