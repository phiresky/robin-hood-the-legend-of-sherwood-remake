import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

// Review-only candidate: give the raised platform its own walkable receiver.
// Material regions remain attached to the physical volume; surrounding terrain
// must meet its authored height instead of moving the platform during export.
const id = "leicester-watermill";
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json")).assets;
const entry = index.find((entry) => entry.id === id);
assert.ok(entry);
const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
assert.equal(hash(bytes), "b008913a237417aadef280b0ac3ab8835ea69482aeefa81fc0055833c153c01b");
assert.equal(entry.descriptor_sha256, hash(bytes));
const modelSha256 = hash(await fs.readFile(`library/3d-assets/${entry.model}`));
assert.equal(modelSha256, "ba0b9fc1d5047e0bffc876e357408270651414e3d31c8190bee86466a133dc11");
const descriptor = JSON.parse(bytes);
const gameplay = structuredClone(descriptor.gameplay);
const receiver = gameplay.projectionReceivers.find((item) => item.id === "building-076-receiver");
assert.ok(receiver);
const part = descriptor.parts.find((part) => part.node === receiver.volume);
assert.ok(part && part.node === receiver.node);
const shape = part.obstacle_local_game;
assert.ok(shape.solid && shape.points.every((point) => point.z_top === shape.points[0].z_top));
assert.equal(
  gameplay.materials.filter((material) => material.obstacles?.includes(part.node)).length,
  2,
);
assert.equal(gameplay.surfaces.length, 0);
gameplay.surfaces.push({
  id: "watermill-platform-floor",
  node: part.node,
  navigationRegion: "watermill-platform",
  projectionVolume: part.node,
  polygon: shape.points.map((point) => [point.x, point.y]),
  height: shape.points[0].z_top,
  preserveMovementPrecision: true,
  navigationJoins: shape.points.map((point, i) => {
    const next = shape.points[(i + 1) % shape.points.length];
    return [
      [point.x, point.y, point.z_top],
      [next.x, next.y, next.z_top],
    ];
  }),
});
gameplay.projectionReceivers = gameplay.projectionReceivers.filter((item) => item !== receiver);
const upperDoors = gameplay.interiors
  .flatMap((room) => room.doors)
  .filter((door) => door.outside[2] === shape.points[0].z_top);
assert.equal(upperDoors.length, 1);
const door = upperDoors[0];
const beforeMiddle = structuredClone(door.middle);
const platform = gameplay.surfaces[0].polygon;
const inside = ([x, y]) => {
  let result = false;
  for (let i = 0, j = platform.length - 1; i < platform.length; j = i++) {
    const a = platform[i],
      b = platform[j];
    if (a[1] > y !== b[1] > y && x < ((b[0] - a[0]) * (y - a[1])) / (b[1] - a[1]) + a[0])
      result = !result;
  }
  return result;
};
assert.ok(!inside(beforeMiddle) && inside(door.outside));
const delta = door.outside.map((value, index) => value - beforeMiddle[index]);
const at = (t) => beforeMiddle.map((value, index) => value + t * delta[index]);
let lower = 0,
  upper = 1;
for (let i = 0; i < 50; i++) {
  const t = (lower + upper) / 2;
  if (inside(at(t))) upper = t;
  else lower = t;
}
// The handoff must remain on the platform after integer waypoint rounding.
door.middle = at(Math.min(1, upper + 1.5 / Math.hypot(delta[0], delta[1])));
gameplay.draft ??= { issues: [] };
gameplay.draft.issues.push(
  "Unpublished physical watermill platform candidate: moved terrain connections, actor routes, jumps and rendered contact require review. Near-horizontal top mesh within 0.1 game units leaves 3.431 square game units of the authored platform footprint unsupported.",
);
gameplay.draft.issues.push(
  "Unpublished upper doorway contact moved onto the authored platform; rendered threshold alignment requires review.",
);
validateAssetGameplay(gameplay, descriptor);
const output = await fs.mkdtemp("work/map-compile/watermill-platform-floor-");
await fs.writeFile(
  `${output}/edits.json`,
  JSON.stringify([{ asset: id, descriptorSha256: hash(bytes), modelSha256, gameplay }]),
);
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({
    asset: id,
    replacedReceiver: receiver,
    surface: gameplay.surfaces[0],
    threshold: { before: beforeMiddle, after: door.middle, inset: 1.5 },
    modelSha256,
  }),
);
console.log(output);
