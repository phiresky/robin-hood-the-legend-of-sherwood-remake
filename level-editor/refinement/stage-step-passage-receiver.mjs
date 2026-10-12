// Candidate repair only; native passage traversal must precede publication.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const asset = "lincoln-south-wall-walk-step";
const root = `library/3d-assets/lincoln/${asset}`;
const bytes = await fs.readFile(`${root}/asset.json`);
const descriptor = JSON.parse(bytes);
const gameplay = descriptor.gameplay;
const door = gameplay.doors.find((door) => door.id === "passage-0/door-0");
assert.ok(door?.insideAnchor && door.type === 0);
const before = structuredClone(door);
// Binding and destination must describe the same actual receiving position.
door.inside = [...door.insideAnchor];
delete door.insideAnchor;
validateAssetGameplay(gameplay, descriptor);
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const output = await fs.mkdtemp("work/map-compile/step-passage-receiver-");
await fs.writeFile(
  `${output}/edits.json`,
  JSON.stringify([
    {
      asset,
      descriptorSha256: hash(bytes),
      modelSha256: hash(await fs.readFile(`${root}/${descriptor.model}`)),
      gameplay,
    },
  ]),
);
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({ asset, before, after: door, status: "candidate awaiting native traversal" }),
);
console.log(output);
