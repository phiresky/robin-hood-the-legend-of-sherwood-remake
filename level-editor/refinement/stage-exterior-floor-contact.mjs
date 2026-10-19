// Author one reviewed exterior edge as an asset-local navigation socket.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const [stage, directory, surfaceId, edgeText] = process.argv.slice(2);
const edge = Number(edgeText);
assert.ok(stage && directory && surfaceId && Number.isInteger(edge));
const bytes = await fs.readFile(`library/${directory}/asset.json`);
const asset = JSON.parse(bytes);
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`));
const edit = edits.find((entry) => entry.asset === asset.id);
assert.ok(edit);
const hash = (data) => createHash("sha256").update(data).digest("hex");
assert.equal(hash(bytes), edit.descriptorSha256);
assert.equal(hash(await fs.readFile(`library/${directory}/${asset.model}`)), edit.modelSha256);
const surface = edit.gameplay.surfaces.find((entry) => entry.id === surfaceId);
assert.ok(surface && edge >= 0 && edge < surface.polygon.length);
const before = structuredClone(surface);
const socket = [edge, (edge + 1) % surface.polygon.length].map((i) => [
  ...surface.polygon[i],
  Array.isArray(surface.height) ? surface.height[i] : surface.height,
]);
assert.ok(
  !surface.navigationJoins?.some((join) => JSON.stringify(join) === JSON.stringify(socket)),
);
surface.navigationRegion ??= surface.id;
surface.navigationJoins = [...(surface.navigationJoins ?? []), socket];
surface.preserveMovementPrecision = true;
validateAssetGameplay(edit.gameplay, asset);
const output = await fs.mkdtemp("work/needs-review/map-compile/exterior-floor-contact-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify(
    {
      scope: "unpublished-exterior-socket-requires-geometry-and-native-traversal-review",
      stage,
      changes: [{ asset: asset.id, before, after: surface }],
    },
    null,
    2,
  ),
);
console.log(output);
