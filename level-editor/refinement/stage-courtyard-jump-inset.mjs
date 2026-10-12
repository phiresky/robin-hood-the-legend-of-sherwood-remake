// Unpublished authoring candidate: move the ledge inward on its narrow wall walk.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const asset = "nottingham-castle-west-courtyard-wall";
const root = `library/3d-assets/nottingham/${asset}`;
const bytes = await fs.readFile(`${root}/asset.json`);
const descriptor = JSON.parse(bytes);
const segment = descriptor.gameplay.jumpSegments.find((entry) => entry.id === "jump-pair-7-side-0");
assert.ok(segment);
const before = structuredClone(segment);
// Three local world-Y units move the approach off the outer receiving boundary;
// the socket and the edge's direction/height remain independent of its neighbor.
for (const point of [segment.edge.a, segment.edge.b]) point[1] -= 3;
validateAssetGameplay(descriptor.gameplay, descriptor);
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const output = await fs.mkdtemp("work/needs-review/map-compile/courtyard-jump-inset-");
await fs.writeFile(
  `${output}/edits.json`,
  JSON.stringify([
    {
      asset,
      descriptorSha256: hash(bytes),
      modelSha256: hash(await fs.readFile(`${root}/${descriptor.model}`)),
      gameplay: descriptor.gameplay,
    },
  ]),
);
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({
    asset,
    before,
    after: segment,
    status: "candidate awaiting native and moved-placement checks",
  }),
);
console.log(output);
