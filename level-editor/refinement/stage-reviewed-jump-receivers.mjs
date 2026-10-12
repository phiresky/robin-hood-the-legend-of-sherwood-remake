import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { createRequire } from "node:module";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { pointInGameplayPolygon as inside } from "../shared/src/navigation-anchor.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const polygonClipping = createRequire(new URL("../shared/src/jump-clearance.ts", import.meta.url))(
  "polygon-clipping",
);

// Reviewed asset-local ledge/face associations. Digests prevent applying these
// authoring decisions to changed geometry. Export never reads this recipe.
const map = process.argv[2] ?? "york";
assert.ok(["york", "nottingham"].includes(map) && process.argv.length <= 3);
const york = [
  [
    "york-bridge-square-scaffolded-corner-house",
    "0eed6df64a0cdf305108d9bdd2ef588af913fdc3a6bce117d44095c3fa8bd0fb",
    "3e9a64b7556c5ccb0e4c05cfd8866f186d2d86249b7448aab75db0bec555fdeb",
    [["jump-pair-49/0", "jump-zone-49", "building-138-walk-0"]],
  ],
  [
    "york-castle-southwest-bastion",
    "1363fe8e18d3ad31e5e65f25592ffaab22cd95218c6f12e756dc4205edd8d600",
    "b78a9f96782ef870f73f5904f6d6c4eb960a06087b9f5faf7e9a8b4c59cface5",
    [["jump-pair-67/1", "jump-zone-77", "building-767-walk-0"]],
  ],
  [
    "york-central-lane-polygonal-corner-house",
    "deef7ad11889a1faee37b2edb2ddad0c99c6955c5b1d96efa6d5b9f6f3e5bd07",
    "5291e2956aab3d5be0e8630334ece0579b8ebaa9f5148ce475dc382c4f5c0ece",
    [
      ["jump-pair-52-side-1", "jump-zone-64", "building-158-walk-0"],
      ["jump-pair-53-side-0", "jump-zone-64", "building-160-walk-0"],
    ],
  ],
  [
    "york-central-south-golden-timber-house",
    "229f169ee9fd4d3bf81ffa94616b19c3aec591a85f49ade1ddf5205b6ad84331",
    "f6fd8cf6b401c5c549f56edbce1a4b6f2ffde41840a9fd8dcd80c51c860e65df",
    [["jump-pair-43-side-1", "jump-zone-43", "building-339-walk-0"]],
  ],
  [
    "york-market-southwest-central-hipped-house",
    "198fd5ad0c1943c2de203ef057f5c76851c86a85bfc849e88622fb6f72de8ab5",
    "73b98d74963a8800582f99011c01448a8a11d2761f015b0e235d79d1e4833574",
    [
      ["jump-pair-17-side-0", "jump-zone-22", "building-484-walk-0"],
      ["jump-pair-18-side-1", "jump-zone-22", "building-484-walk-0"],
    ],
  ],
  [
    "york-southeast-lane-eastern-timber-house",
    "ea593511582e23d33d7430fe9342480782a1166034b969bc85c2759d92d1bfdb",
    "99c80272b761701720c3e2add927b38783b9c80751d328d1b164c036de67df45",
    [["jump-pair-36-side-0", "jump-zone-35", "building-384-walk-0"]],
  ],
  [
    "york-west-bridge-street-arcade-house",
    "2984c80d599cf28c8dba61780580df2c2f422e5f17391e98992c20d281d14633",
    "90e089ad15476ea4e5289a891c75451dae50130f7b0df742af070cc0f63268c7",
    [["jump-pair-25-side-1", "jump-zone-29", "building-200-walk-0"]],
  ],
  [
    "york-west-town-lane-rear-timber-house",
    "7560681ffe92837c255d3eff7aefb1efc42f0ca8c3383bb6bde35e2418d3d02e",
    "caa9fea7c65dda2c165ea0f24e7cdab350ab12b24345d4fbe83e814598b79dd2",
    [["jump-pair-55-side-0", "jump-zone-85", "building-594-walk-0"]],
  ],
];
const reviewed =
  map === "york"
    ? york
    : JSON.parse(
        await fs.readFile(
          new URL("./reviewed-nottingham-jump-receivers.json", import.meta.url),
          "utf8",
        ),
      );
// The Nottingham review uses this explicit authoring precision to join nearly
// coincident clipping endpoints. The installed polygons carry the result.
const clipPoint = (point) =>
  map === "nottingham" ? point.map((value) => Math.round(value * 1e6) / 1e6) : point;
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8"));
const edits = [],
  reviews = [];
for (const [id, descriptorHash, modelHash, rules] of reviewed) {
  const entry = index.assets.find((item) => item.id === id);
  assert.ok(entry, id);
  const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
  assert.equal(hash(bytes), descriptorHash, `${id}: reviewed descriptor changed`);
  assert.equal(entry.descriptor_sha256, descriptorHash);
  assert.equal(hash(await fs.readFile(`library/3d-assets/${entry.model}`)), modelHash);
  const asset = JSON.parse(bytes),
    gameplay = asset.gameplay;
  const originalZones = structuredClone(gameplay.jumpZones);
  for (const [edgeId, zoneId, surfaceId] of rules) {
    const segment = gameplay.jumpSegments?.find((item) => item.id === edgeId);
    const [pairId, side] = edgeId.split("/");
    const edge =
      segment?.edge ?? gameplay.jumpPairs?.find((item) => item.id === pairId)?.edges[Number(side)];
    assert.ok(edge, `${id}: missing ${edgeId}`);
    assert.equal(edge.zone, zoneId);
    const zone = originalZones.find((item) => item.id === zoneId);
    const surface = gameplay.surfaces.find((item) => item.id === surfaceId);
    assert.ok(zone && surface);
    assert.equal(surface.navigationHeight, undefined);
    assert.equal(surface.holes?.length ?? 0, 0);
    const vertices = surface.polygon.map(([x, y], i) => {
      const z = Array.isArray(surface.height) ? surface.height[i] : surface.height;
      return [x, y - z, z];
    });
    const plane = heightPlane(vertices);
    const midpoint = edge.a.map((value, i) => (value + edge.b[i]) / 2);
    const point = [midpoint[0], midpoint[1] - midpoint[2]];
    const height = planeHeight(plane, point);
    assert.ok(inside(point, vertices, true) && Math.abs(height - midpoint[2]) < 1.1);
    const clipped = polygonClipping.intersection(
      [zone.polygon.map(([x, y, z]) => clipPoint([x, y - z]))],
      [vertices.map(([x, y]) => clipPoint([x, y]))],
    );
    const regions = clipped.filter((region) => inside(point, region[0], true));
    assert.equal(regions.length, 1);
    assert.equal(regions[0].length, 1);
    const polygon = regions[0][0].slice(0, -1).map(([x, y]) => {
      const z = planeHeight(plane, [x, y]);
      return [x, y + z, z];
    });
    const replacement = {
      ...zone,
      id: `${zoneId}/receiver/${surfaceId}`,
      node: surface.node,
      anchor: [point[0], point[1] + height, height],
      polygon,
    };
    const existing = gameplay.jumpZones.find((item) => item.id === replacement.id);
    if (existing) {
      assert.deepEqual(existing.polygon, polygon);
      assert.equal(existing.node, replacement.node);
    } else gameplay.jumpZones.push(replacement);
    edge.zone = replacement.id;
    reviews.push({ asset: id, edge: edgeId, before: zone, after: existing ?? replacement });
  }
  const used = new Set([
    ...(gameplay.jumpPairs ?? []).flatMap((pair) => pair.edges.map((edge) => edge.zone)),
    ...(gameplay.jumpSegments ?? []).map((segment) => segment.edge.zone),
  ]);
  gameplay.jumpZones = gameplay.jumpZones.filter((zone) => used.has(zone.id));
  gameplay.draft.issues.push(
    "Receiver-specific jump zones follow each authored ledge face. Narrow rotated navigation regions, click coverage and complete traversal remain under review.",
  );
  validateAssetGameplay(gameplay, asset);
  edits.push({ asset: id, descriptorSha256: descriptorHash, modelSha256: modelHash, gameplay });
}
const output = await fs.mkdtemp("work/map-compile/reviewed-jump-receivers-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(`${output}/review.json`, JSON.stringify(reviews));
console.log(output);
