// Stage asset-local click coverage behind an existing ground landing edge.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { fixedPolygonBoolean } from "../shared/src/fixed-polygon-boolean.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const id = "nottingham-southeast-green-house";
const root = `library/3d-assets/nottingham/${id}`;
const bytes = await fs.readFile(`${root}/asset.json`);
const asset = JSON.parse(bytes);
const pair = asset.gameplay.jumpPairs.find((pair) => pair.id === "jump-pair-18");
const edge = pair.edges.find((edge) => edge.zone === "jump-zone-20");
const zone = asset.gameplay.jumpZones.find((zone) => zone.id === edge.zone);
const before = structuredClone(zone);
assert.ok(zone.polygon.every((point) => Math.abs(point[2] - zone.anchor[2]) < 1e-7));
assert.ok([edge.a, edge.b].every((point) => Math.abs(point[2] - zone.anchor[2]) < 0.01));
const [a, b] = [edge.a, edge.b].map(([x, y]) => [x, y]);
const dx = b[0] - a[0],
  dy = b[1] - a[1],
  length = Math.hypot(dx, dy);
assert.ok(length > 0);
const depth = 8;
const margin = 4;
const offset = [(dy / length) * depth, (-dx / length) * depth];
const tangent = [dx / length, dy / length];
const start = a.map((n, i) => n - tangent[i] * margin);
const end = b.map((n, i) => n + tangent[i] * margin);
const edgeMargin = 2;
const band = [
  start.map((n, i) => n - (offset[i] * edgeMargin) / depth),
  end.map((n, i) => n - (offset[i] * edgeMargin) / depth),
  end.map((n, i) => n + offset[i]),
  start.map((n, i) => n + offset[i]),
];
const union = fixedPolygonBoolean("union", [zone.polygon.map(([x, y]) => [x, y])], [[band]]);
assert.equal(union.length, 1);
assert.equal(union[0].length, 1);
zone.polygon = union[0][0].map(([x, y]) => [x, y, zone.anchor[2]]);
validateAssetGameplay(asset.gameplay, asset);
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const output = await fs.mkdtemp("work/map-compile/ground-jump-click-band-");
await fs.writeFile(
  `${output}/edits.json`,
  JSON.stringify([
    {
      asset: id,
      descriptorSha256: hash(bytes),
      modelSha256: hash(await fs.readFile(`${root}/${asset.model}`)),
      gameplay: asset.gameplay,
    },
  ]),
);
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({
    id,
    depth,
    margin,
    edgeMargin,
    before,
    after: zone,
    edge,
    status: "unpublished: requires full-map and moved-placement traversal checks",
  }),
);
console.log(output);
