// Review subpixel seam repair without changing published assets.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const id = "york-southwest-market-north-house";
const scene = JSON.parse(await fs.readFile("library/scenes/york.rhlos-map.json"));
const ref = scene.assetSources.find((source) => source.id === id);
assert.ok(ref);
const bytes = await fs.readFile(`library/${ref.descriptor}`);
assert.equal(createHash("sha256").update(bytes).digest("hex"), ref.descriptor_sha256);
const asset = JSON.parse(bytes);
const gameplay = structuredClone(asset.gameplay);
const main = gameplay.surfaces.find((surface) => surface.id === "building-514-walk-0");
const wing = gameplay.surfaces.find((surface) => surface.id === "building-516-walk-0");
assert.ok(main && wing && Array.isArray(main.height) && Array.isArray(wing.height));
assert.equal(main.navigationRegion, wing.navigationRegion);
const changes = [];
for (const [index, point] of wing.polygon.entries()) {
  const vertex = [...point, wing.height[index]];
  const matches = main.polygon.flatMap((candidate, other) => {
    const target = [...candidate, main.height[other]];
    const distance = Math.hypot(...target.map((value, axis) => value - vertex[axis]));
    return distance < 0.0001 ? [{ target, distance }] : [];
  });
  assert.ok(matches.length <= 1);
  if (!matches.length) continue;
  const { target, distance } = matches[0];
  wing.polygon[index] = target.slice(0, 2);
  wing.height[index] = target[2];
  changes.push({ index, before: vertex, after: target, distance });
}
assert.equal(changes.length, 2, "Expected exactly the two shared seam endpoints");
validateAssetGameplay(gameplay, asset);
const output = await fs.mkdtemp("work/needs-review/map-compile/york-market-roof-seam-");
await fs.writeFile(
  `${output}/edits.json`,
  JSON.stringify([
    {
      asset: id,
      descriptorSha256: ref.descriptor_sha256,
      modelSha256: ref.model_sha256,
      gameplay,
    },
  ]),
);
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({ scope: "unpublished-shared-seam-candidate", changes }),
);
console.log(output);
