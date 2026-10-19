// Keep the two sides of the terrace gate as independently connected floors.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const [stage] = process.argv.slice(2);
assert.ok(stage, "Supply staged western terrace floors");
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`));
const edit = edits.find((entry) => entry.asset === "lincoln-west-tower-terrace");
assert.ok(edit);
const root = `library/3d-assets/lincoln/${edit.asset}`;
const bytes = await fs.readFile(`${root}/asset.json`);
const hash = (data) => createHash("sha256").update(data).digest("hex");
assert.equal(hash(bytes), edit.descriptorSha256);
const descriptor = JSON.parse(bytes);
assert.equal(hash(await fs.readFile(`${root}/${descriptor.model}`)), edit.modelSha256);
const floor = edit.gameplay.surfaces.find((surface) => surface.node === "building-069");
assert.ok(floor && floor.navigationRegion === "lower-terrace");
assert.ok(!floor.navigationJoins?.length);
floor.navigationRegion = "lower-terrace-outside-gate";
edit.gameplay.draft.issues.push(
  "Unpublished gate floor separation: verify locked/open passage controls and neighboring routes at all placements.",
);
validateAssetGameplay(edit.gameplay, descriptor);
const output = await fs.mkdtemp("work/needs-review/map-compile/west-terrace-gate-regions-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({
    stage,
    asset: edit.asset,
    floor: floor.id,
    navigationRegion: floor.navigationRegion,
  }),
);
console.log(output);
