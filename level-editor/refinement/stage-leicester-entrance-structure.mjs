// Restore explicit doorway structure without altering walking or render geometry.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { prepareCorridorStates } from "../shared/src/compile-navigation-graph.ts";

const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const reviewed = [
  [
    "north-village-cottage",
    "de2d38272c333baf63c4a6ba91ee49f74690262fcbbf618856ab9e8783ae0c61",
    [0],
    [],
    ["building-027-ground-blocker-62-0-0"],
  ],
  [
    "northeast-gabled-house",
    "75e823050da859cc057d53cf1936ee4d9f4159f634c1c99c6bd0cbc524b2ed21",
    [0],
    [],
    ["building-009-ground-blocker-68-0-0"],
  ],
  [
    "northeast-longhouse",
    "08505dd9c1ecfde8815bc25ed51a232850a64366b9ce801aab41e59223b93376",
    [0],
    [],
    ["building-003-ground-blocker-73-0-0"],
  ],
  [
    "south-hall",
    "45a2c8618e2cbbc22d1f0ecdff6f258b7bbd64bc648220156fd43861d0e02194",
    [0],
    ["building-105"],
    [],
  ],
  ["watermill", "1698ac81ef7d101986e6809e0902538a4b02d15e19f4f57bcf98c9f60811138c", [1, 2], [], []],
];
const edits = [];
for (const [name, expected, indices, volumes, blockers] of reviewed) {
  const asset = `leicester-${name}`;
  const directory = `library/3d-assets/leicester/${asset}`;
  const bytes = await fs.readFile(`${directory}/asset.json`);
  assert.equal(hash(bytes), expected);
  const descriptor = JSON.parse(bytes);
  const before = structuredClone(descriptor.gameplay);
  for (const index of indices) {
    const door = descriptor.gameplay.interiors[0].doors[index];
    assert.equal(door.id, `interior-0/door-${index}`);
    const doorVolumes =
      name === "watermill" ? [index === 1 ? "building-077" : "building-078"] : volumes;
    for (const id of doorVolumes) {
      const part = descriptor.parts.find((part) => part.node === id);
      assert.ok(part?.obstacle_local_game?.solid);
      const points = part.obstacle_local_game.points.map((point) => [point.x, point.y]);
      assert.equal(
        prepareCorridorStates([{ state_id: 0, polygon: { points } }])(
          door.outside.slice(0, 2),
          door.middle.slice(0, 2),
        ).length,
        0,
        "Declared structure must cross the animated doorway",
      );
    }
    for (const id of blockers)
      assert.equal(
        descriptor.gameplay.movementBlockers.find((row) => row.id === id)?.node,
        door.node,
      );
    door.passageStructure = { volumes: doorVolumes, blockers };
  }
  validateAssetGameplay(descriptor.gameplay, descriptor);
  const unannotated = structuredClone(descriptor.gameplay);
  for (const index of indices) delete unannotated.interiors[0].doors[index].passageStructure;
  assert.deepEqual(unannotated, before);
  edits.push({
    asset,
    descriptorSha256: hash(bytes),
    modelSha256: hash(await fs.readFile(`${directory}/${descriptor.model}`)),
    gameplay: descriptor.gameplay,
  });
}
const output = await fs.mkdtemp("work/needs-review/map-compile/leicester-entrance-structure-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
console.log(output);
