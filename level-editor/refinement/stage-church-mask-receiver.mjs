import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { parseProjectionAssetDescriptor } from "../shared/src/index.ts";

// Author receiver selection from existing asset-local mask and ground metadata.
// This stages a reviewable edit; configure-surface-jumps performs publication.
const catalog = JSON.parse(
  await fs.readFile("refinement/catalogs/leicester-church-mask-receiver.json", "utf8"),
);
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8"));
const entry = index.assets.find((asset) => asset.id === catalog.asset);
assert.ok(entry, "Missing tower asset");
const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`, "utf8");
const digest = createHash("sha256").update(bytes).digest("hex");
assert.equal(digest, entry.descriptor_sha256, "Asset index is stale");
const descriptor = JSON.parse(bytes);
const mask = descriptor.gameplay.masks.find((mask) => mask.id === catalog.mask);
assert.ok(mask, "Missing selected mask");
assert.deepEqual(catalog.receiverSegment[0], mask.anchor);
assert.deepEqual(catalog.receiverSegment[1].slice(0, 2), mask.anchor.slice(0, 2));
assert.equal(catalog.receiverSegment[1][2], descriptor.gameplay.placementGroundHeight);
assert.ok(!mask.receiverPoints && !mask.receiverPolyline && !mask.receiverPolylines);
if (mask.receiverSegment) assert.deepEqual(mask.receiverSegment, catalog.receiverSegment);
mask.receiverSegment = catalog.receiverSegment;
parseProjectionAssetDescriptor(descriptor);
const output = await fs.mkdtemp("work/map-compile/church-mask-receiver-");
await fs.writeFile(`${output}/asset.json`, JSON.stringify(descriptor) + "\n");
await fs.writeFile(
  `${output}/edits.json`,
  JSON.stringify([
    { asset: catalog.asset, descriptorSha256: digest, gameplay: descriptor.gameplay },
  ]),
);
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({ ...catalog, descriptorSha256: digest }),
);
console.log(output);
