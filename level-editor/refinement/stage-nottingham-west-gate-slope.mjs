// Preserve a sloped doorway's authored floor before final movement-grid rounding.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const [stage] = process.argv.slice(2);
assert.ok(stage, "Supply the staged Nottingham doorway definitions");
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`));
const edit = edits.find((entry) => entry.asset === "nottingham-castle-gate-west-tower");
assert.ok(edit);
const scene = JSON.parse(await fs.readFile("library/scenes/nottingham.rhlos-map.json"));
const source = scene.assetSources.find((entry) => entry.id === edit.asset);
assert.ok(source);
const bytes = await fs.readFile(`library/${source.descriptor}`);
assert.equal(createHash("sha256").update(bytes).digest("hex"), edit.descriptorSha256);
assert.equal(
  createHash("sha256")
    .update(await fs.readFile(`library/${source.model}`))
    .digest("hex"),
  edit.modelSha256,
);
const surface = edit.gameplay.surfaces.find((entry) => entry.id === "building-330-walk-0");
assert.ok(surface);
surface.preserveMovementPrecision = true;
const descriptor = JSON.parse(bytes);
const door = edit.gameplay.interiors
  .find((room) => room.id === "interior-0")
  ?.doors.find((entry) => entry.id === "interior-0/door-0");
assert.ok(door?.passageStructure);
assert.deepEqual(door.passageStructure.volumes, ["building-329", "building-330", "building-331"]);
assert.ok(descriptor.parts.find((part) => part.node === "building-332")?.obstacle_local_game.solid);
// The animated entrance crosses the tower's fourth structural side at oblique
// placements. This declaration leaves its ordinary walking collision intact.
door.passageStructure.volumes.push("building-332");
validateAssetGameplay(edit.gameplay, descriptor);
const output = await fs.mkdtemp("work/needs-review/map-compile/west-gate-slope-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits, null, 2));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify(
    {
      input: stage,
      asset: edit.asset,
      surface: surface.id,
      door: door.id,
      addedStructure: "building-332",
      scope: "unpublished-slope-precision-candidate-requires-placement-verification",
    },
    null,
    2,
  ),
);
console.log(output);
