import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { createRequire } from "node:module";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

// Explicit authoring candidate; mesh and placed actor checks precede publication.
const [source] = process.argv.slice(2);
assert.ok(source, "Provide the reviewed central-oak ladder seam stage");
const require = createRequire(new URL("../shared/package.json", import.meta.url));
const clipping = require("polygon-clipping");
const edits = JSON.parse(await fs.readFile(`${source}/edits.json`, "utf8"));
assert.equal(edits.length, 1);
const edit = edits[0];
assert.equal(edit.asset, "sherwood-central-oak-platform");
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
const entry = index.find((entry) => entry.id === edit.asset);
const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
assert.equal(createHash("sha256").update(bytes).digest("hex"), edit.descriptorSha256);
const original = JSON.parse(bytes);
const surface = edit.gameplay.surfaces.find((surface) => surface.id === "building-088-walk-0");
const before = original.gameplay.surfaces.find((candidate) => candidate.id === surface.id);
assert.equal(surface.projectionVolume, before.projectionVolume);
assert.equal(surface.holes.length, 0);
assert.ok(surface.height.every((height) => height === 220.001));
const patches = clipping.difference([surface.polygon], [before.polygon]);
assert.equal(patches.length, 1, "Expected one connected landing extension");
assert.equal(patches[0].length, 1, "Landing extension must not contain holes");
const clearance = {
  id: "central-oak-lower-platform-seam-clearance",
  node: surface.node,
  polygon: patches[0][0].slice(0, -1),
  height: 220.001,
  holes: [],
  preserveMovementPrecision: true,
};
assert.ok(!edit.gameplay.movementClearances.some((value) => value.id === clearance.id));
edit.gameplay.movementClearances.push(clearance);
const receiver = {
  id: "central-oak-lower-platform-receiver",
  node: surface.node,
  shape: {
    points: surface.polygon.map(([x, y]) => ({ x, y, z_bottom: 220.001, z_top: 220.001 })),
    opaque: false,
    solid: false,
    mouse: true,
    show_shadow_polygon: false,
    default_material: original.parts.find((part) => part.node === before.projectionVolume)
      .obstacle_local_game.default_material,
  },
};
assert.ok(!edit.gameplay.volumes.some((volume) => volume.id === receiver.id));
edit.gameplay.volumes.push(receiver);
surface.projectionVolume = receiver.id;
edit.gameplay.draft ??= { issues: [] };
edit.gameplay.draft.issues.push(
  "Central oak ladder retains incomplete visible mesh coverage; its corrected lower platform seam extends up to 1.643 game units beyond the mesh. Rendered actor-compositing review remains required.",
);
validateAssetGameplay(edit.gameplay, original);
const review = JSON.parse(await fs.readFile(`${source}/review.json`, "utf8"));
review.changes.push({ clearance: clearance.id, after: clearance });
review.changes.push({ receiver: receiver.id, before: before.projectionVolume, after: receiver });
const output = await fs.mkdtemp("work/map-compile/sherwood-oak-landing-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(`${output}/review.json`, JSON.stringify(review, null, 2));
console.log(JSON.stringify({ output, clearance: clearance.polygon }));
