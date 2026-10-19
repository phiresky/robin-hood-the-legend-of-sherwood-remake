// Stage collision-audit candidates for placement/native testing; never publish them.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const [audit] = process.argv.slice(2);
assert.ok(audit, "Supply a local-door-structure audit directory");
const proposals = JSON.parse(await fs.readFile(`${audit}/proposals.json`));
assert.equal(proposals.scope, "unreviewed-local-collision-candidates-not-traversal-proof");
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const staged = proposals.gameplayStage
  ? JSON.parse(await fs.readFile(`${proposals.gameplayStage}/edits.json`))
  : [];
const stagedByAsset = new Map(staged.map((edit) => [edit.asset, edit]));
const assets = new Map();
for (const row of proposals.results) {
  if (!row.proposed.volumes.length && !row.proposed.blockers.length) continue;
  if (!assets.has(row.asset)) {
    const bytes = await fs.readFile(`library/${row.descriptor}`);
    assert.equal(hash(bytes), row.descriptorSha256);
    assert.equal(hash(await fs.readFile(`library/${row.model}`)), row.modelSha256);
    const descriptor = JSON.parse(bytes);
    assert.equal(descriptor.id, row.asset);
    const edit = stagedByAsset.get(row.asset);
    if (edit) {
      assert.equal(edit.descriptorSha256, row.descriptorSha256);
      if (edit.modelSha256 !== undefined) assert.equal(edit.modelSha256, row.modelSha256);
      descriptor.gameplay = structuredClone(edit.gameplay);
    }
    if (row.gameplaySha256 !== undefined)
      assert.equal(hash(JSON.stringify(descriptor.gameplay)), row.gameplaySha256);
    assets.set(row.asset, {
      descriptor,
      before: structuredClone(descriptor.gameplay),
      row,
      doors: [],
    });
  }
  const entry = assets.get(row.asset);
  const door = entry.descriptor.gameplay.interiors
    .find((room) => room.id === row.room)
    ?.doors.find((door) => door.id === row.door);
  assert.ok(door);
  if (entry.doors.includes(row.door)) {
    assert.deepEqual(door.passageStructure, row.proposed);
    continue;
  }
  assert.equal(door.passageStructure, undefined, "Do not overwrite previously authored structure");
  door.passageStructure = row.proposed;
  entry.doors.push(row.door);
}
const edits = staged.filter((edit) => !assets.has(edit.asset));
for (const [asset, entry] of assets) {
  validateAssetGameplay(entry.descriptor.gameplay, entry.descriptor);
  const unannotated = structuredClone(entry.descriptor.gameplay);
  for (const room of unannotated.interiors)
    for (const door of room.doors) if (entry.doors.includes(door.id)) delete door.passageStructure;
  assert.deepEqual(unannotated, entry.before, "Candidate must leave all existing geometry intact");
  edits.push({
    asset,
    descriptorSha256: entry.row.descriptorSha256,
    modelSha256: entry.row.modelSha256,
    gameplay: entry.descriptor.gameplay,
  });
}
const output = await fs.mkdtemp("work/needs-review/map-compile/local-door-structure-stage-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify(
    {
      scope: "unpublished-collision-candidates-require-placement-native-and-browser-verification",
      audit,
      ...(proposals.gameplayStage ? { gameplayStage: proposals.gameplayStage } : {}),
      assets: [...assets].map(([asset, entry]) => ({ asset, doors: entry.doors })),
    },
    null,
    2,
  ),
);
console.log(output);
console.log(`${edits.length} candidate assets`);
