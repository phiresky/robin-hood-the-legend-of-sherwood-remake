// Stage parallel roof spans tangent to both receiving planes.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const [stage] = process.argv.slice(2);
assert.ok(stage, "Supply the unpublished inset/selection stage");
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`));
const scene = JSON.parse(await fs.readFile(`${stage}/scene.json`));
const sides = [];
for (const [id, surfaceId, segmentId] of [
  ["york-central-south-red-timber-house", "building-323-walk-0", "jump-pair-45-side-0"],
  ["york-central-south-rear-timber-house", "building-331-walk-0", "jump-pair-45-side-1"],
]) {
  const edit = edits.find((e) => e.asset === id);
  const ref = scene.assetSources.find((r) => r.id === id);
  assert.ok(edit && ref && edit.descriptorSha256 === ref.descriptor_sha256);
  assert.equal(edit.modelSha256, ref.model_sha256);
  const bytes = await fs.readFile(`library/${ref.descriptor}`);
  assert.equal(createHash("sha256").update(bytes).digest("hex"), ref.descriptor_sha256);
  const descriptor = JSON.parse(bytes);
  const surface = edit.gameplay.surfaces.find((s) => s.id === surfaceId);
  const segment = edit.gameplay.jumpSegments.find((s) => s.id === segmentId);
  assert.ok(surface && segment && Array.isArray(surface.height));
  const plane = heightPlane(surface.polygon.map((p, i) => [...p, surface.height[i]]));
  sides.push({ edit, descriptor, surface, segment, plane, before: structuredClone(segment) });
}
const [first, second] = sides;
let direction = [first.plane[1] - second.plane[1], second.plane[0] - first.plane[0]];
const length = Math.hypot(...direction);
assert.ok(length > 1e-6);
direction = direction.map((n) => n / length);
const original = first.segment.edge.b.map((n, i) => n - first.segment.edge.a[i]);
if (direction[0] * original[0] + direction[1] * original[1] < 0)
  direction = direction.map((n) => -n);
for (const [side, entry] of sides.entries()) {
  const edge = entry.segment.edge;
  const center = edge.a.map((n, i) => (n + edge.b[i]) / 2);
  const sign = side === 0 ? 1 : -1;
  for (const [key, end] of [
    ["a", -1],
    ["b", 1],
  ]) {
    const point = [
      center[0] + direction[0] * sign * end * 1.5,
      center[1] + direction[1] * sign * end * 1.5,
    ];
    assert.ok(pointInGameplayPolygon(point, entry.surface.polygon, true));
    edge[key] = [...point, planeHeight(entry.plane, point)];
  }
  validateAssetGameplay(entry.edit.gameplay, entry.descriptor);
}
const vectors = sides.map(({ segment: { edge } }) => edge.b.map((n, i) => n - edge.a[i]));
assert.ok(vectors[0].every((n, i) => Math.abs(n + vectors[1][i]) < 1e-6));
const output = await fs.mkdtemp("work/needs-review/map-compile/york-roof-common-span-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(`${output}/scene.json`, JSON.stringify(scene));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({
    stage,
    direction,
    span: 3,
    published: false,
    scope: "common-tangent-candidate-requires-placement-flight-and-mesh-review",
    sides: sides.map(({ edit, before, segment, plane }) => ({
      asset: edit.asset,
      before,
      after: segment,
      plane,
    })),
  }),
);
console.log(output);
