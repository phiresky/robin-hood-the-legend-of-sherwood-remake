// Restore local floor-selection points after the terrace acquires physical floors.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const [stage] = process.argv.slice(2);
assert.ok(stage, "Supply the staged western terrace definitions");
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`));
const scene = JSON.parse(await fs.readFile("library/scenes/lincoln.rhlos-map.json"));
const asset = "lincoln-light-region-5";
assert.ok(!edits.some((edit) => edit.asset === asset));
const source = scene.assetSources.find((source) => source.id === asset);
assert.ok(source);
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const bytes = await fs.readFile(`library/${source.descriptor}`);
assert.equal(hash(bytes), source.descriptor_sha256);
assert.equal(hash(await fs.readFile(`library/${source.model}`)), source.model_sha256);
const descriptor = JSON.parse(bytes);
const gameplay = structuredClone(descriptor.gameplay);
const light = gameplay.lights.find((light) => light.id === "light-5");
assert.ok(light?.receivers?.length === 7);
const changes = [];
for (const [index, inset] of [
  [1, 0],
  [4, 2],
]) {
  const before = light.receivers[index];
  assert.equal(before[2], 0);
  // Retain the projected position when lifting the old ground selector.
  // The eastern selector also needs a two-unit inset from the deck notch.
  const after = [before[0], before[1] + 147.001 + inset, 147.001];
  light.receivers[index] = after;
  changes.push({ index, before, after, inset });
}
// Different receiving heights need their own local contour when the asset turns.
// Preserve the authored screen contour at the initial placement while keeping
// each selector coplanar with the contour that follows its floor after rotation.
const byHeight = new Map();
for (const receiver of light.receivers) {
  const height = receiver[2];
  if (!byHeight.has(height)) byHeight.set(height, []);
  byHeight.get(height).push(receiver);
}
const replacement = [...byHeight].map(([height, receivers]) => ({
  ...light,
  id: height === 147.001 ? light.id : `${light.id}/height-${height}`,
  polygon: light.polygon.map(([x, y, z]) => [x, y - z + height, height]),
  receivers,
}));
gameplay.lights = gameplay.lights.flatMap((entry) => (entry === light ? replacement : [entry]));
gameplay.draft.issues.push(
  "Unpublished physical terrace light selectors: verify receiver coverage in moved assemblies and browser exports.",
);
validateAssetGameplay(gameplay, descriptor);
edits.push({
  asset,
  descriptorSha256: source.descriptor_sha256,
  modelSha256: source.model_sha256,
  gameplay,
});
const output = await fs.mkdtemp("work/needs-review/map-compile/west-terrace-light-receivers-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify(
    {
      stage,
      changes,
      contours: replacement.map(({ id, receivers, polygon }) => ({ id, receivers, polygon })),
    },
    null,
    2,
  ),
);
console.log(output);
