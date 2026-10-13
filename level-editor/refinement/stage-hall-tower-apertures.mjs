// Author adjoining doorway apertures in each reusable asset's own local frame.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { sceneToGame } from "../shared/src/geometry.ts";

const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const directory = "library/3d-assets/nottingham/nottingham-castle-watchtower";
const bytes = await fs.readFile(`${directory}/asset.json`);
assert.equal(hash(bytes), "ecd0494f5a122b13d4d776735da49debb2b8e51bed75787f061357549ce2fa04");
const descriptor = JSON.parse(bytes);
const assemblyBytes = await fs.readFile(
  "library/3d-assets/nottingham-castle-hall-and-watchtower/asset.json",
);
assert.equal(
  hash(assemblyBytes),
  "f0f8b7693bc74144aea80d5e04c9b98a986c311d24f6b89c571b2da6438f6642",
);
const assembly = JSON.parse(assemblyBytes);
assembly.source_origin_scene.forEach((v, i) =>
  assert.ok(Math.abs(v - descriptor.source_origin_scene[i]) < 1e-8),
);
const door = assembly.gameplay.interiors
  .find((room) => room.id === "interior-3")
  .doors.find((door) => door.id === "interior-3/door-2");
assert.ok(door);
assert.ok(Math.abs(door.outside[2] - door.middle[2]) < 1e-8);
// The opening is bounded by the authored shaft mouth. Include a character's
// footprint at the approach; only this tower's solids on this plane are cleared.
const x0 = Math.min(door.outside[0], door.middle[0]) - 14;
const x1 = Math.max(door.outside[0], door.middle[0]) + 14;
const y0 = Math.min(door.outside[1], door.middle[1]) - 10;
const y1 = Math.max(door.outside[1], door.middle[1]) + 10;
const gameplay = structuredClone(descriptor.gameplay);
gameplay.movementClearances.push({
  id: "upper-shaft-doorway-aperture",
  node: "building-538",
  polygon: [
    [x0, y0],
    [x1, y0],
    [x1, y1],
    [x0, y1],
  ],
  height: door.outside[2],
  holes: [],
});
const lower = assembly.gameplay.interiors
  .find((room) => room.id === "interior-3")
  .doors.find((door) => door.id === "interior-3/door-0");
const aperture = (node, offset) => {
  const a = lower.outside.map((v, i) => v + offset[i]);
  const b = lower.middle.map((v, i) => v + offset[i]);
  const left = Math.min(a[0], b[0]) - 14,
    right = Math.max(a[0], b[0]) + 14;
  const top = Math.min(a[1], b[1]) - 10,
    bottom = Math.max(a[1], b[1]) + 10;
  return {
    id: "lower-shaft-doorway-aperture",
    node,
    polygon: [
      [left, top],
      [right, top],
      [right, bottom],
      [left, bottom],
    ],
    height: a[2],
    holes: [],
  };
};
gameplay.movementClearances.push(aperture("building-536", [0, 0, 0]));
assert.ok(process.argv[2], "Supply the standalone hall gameplay candidate");
const [hallEdit] = JSON.parse(await fs.readFile(`${process.argv[2]}/edits.json`));
const hallBytes = await fs.readFile(
  "library/3d-assets/nottingham/nottingham-castle-main-hall/asset.json",
);
assert.equal(hash(hallBytes), hallEdit.descriptorSha256);
const hall = JSON.parse(hallBytes);
const offset = sceneToGame(
  { kind: "oblique-orthographic", elevation_deg: 35 },
  assembly.source_origin_scene.map((v, i) => v - hall.source_origin_scene[i]),
);
hallEdit.gameplay.movementClearances.push(aperture(lower.node, offset));
hallEdit.gameplay.draft.issues.push(
  "Lower shaft mouth clearance is local to this hall; adjacent assets must provide their own opening.",
);
validateAssetGameplay(hallEdit.gameplay, hall);
gameplay.draft.issues.push(
  "Upper shaft aperture is authored locally on the tower; moved doorway contact and rendered wall opening remain under review.",
);
validateAssetGameplay(gameplay, descriptor);
const output = await fs.mkdtemp("work/needs-review/map-compile/watchtower-doorway-clearance-");
await fs.writeFile(
  `${output}/edits.json`,
  JSON.stringify([
    {
      asset: descriptor.id,
      descriptorSha256: hash(bytes),
      modelSha256: hash(await fs.readFile(`${directory}/${descriptor.model}`)),
      gameplay,
    },
    hallEdit,
  ]),
);
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({
    scope: "unpublished-owner-only-doorway-aperture-not-rendered-parity",
    assemblyDescriptorSha256: hash(assemblyBytes),
    door,
  }),
);
console.log(output);
