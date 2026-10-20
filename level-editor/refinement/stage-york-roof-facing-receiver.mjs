// Review a roof connection on the receiving slope facing the other building.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const [
  source,
  rearSurface = "building-331-walk-0",
  redSlideArgument = "0",
  spanArgument = "5",
  interiorFractionArgument = "0",
] = process.argv.slice(2);
assert.ok(source);
assert.ok(["building-330-walk-0", "building-331-walk-0"].includes(rearSurface));
const redSlide = Number(redSlideArgument);
assert.ok(Number.isFinite(redSlide) && Math.abs(redSlide) <= 30);
const span = Number(spanArgument);
assert.ok(Number.isFinite(span) && span >= 1 && span <= 20);
const interiorFraction = Number(interiorFractionArgument);
assert.ok(Number.isFinite(interiorFraction) && interiorFraction >= 0 && interiorFraction <= 1);
const edits = JSON.parse(await fs.readFile(`${source}/edits.json`));
const scene = JSON.parse(await fs.readFile(`${source}/scene.json`));
const sides = [];
for (const [asset, surfaceId, segmentId] of [
  ["york-central-south-red-timber-house", "building-322-walk-0", "jump-pair-45-side-0"],
  ["york-central-south-rear-timber-house", rearSurface, "jump-pair-45-side-1"],
]) {
  const edit = edits.find((entry) => entry.asset === asset);
  const ref = scene.assetSources.find((entry) => entry.id === asset);
  assert.ok(edit && ref);
  assert.equal(edit.descriptorSha256, ref.descriptor_sha256);
  assert.equal(edit.modelSha256, ref.model_sha256);
  const bytes = await fs.readFile(`library/${ref.descriptor}`);
  assert.equal(createHash("sha256").update(bytes).digest("hex"), ref.descriptor_sha256);
  const surface = edit.gameplay.surfaces.find((entry) => entry.id === surfaceId);
  const segment = edit.gameplay.jumpSegments.find((entry) => entry.id === segmentId);
  assert.ok(surface && segment && Array.isArray(surface.height));
  const sharedZone = edit.gameplay.jumpZones.find((entry) => entry.id === segment.edge.zone);
  assert.ok(sharedZone);
  const before = { segment: structuredClone(segment), zone: structuredClone(sharedZone) };
  // Other jump pairs on this asset retain their original receiving slope.
  const zone = structuredClone(sharedZone);
  zone.id = `${sharedZone.id}/alternate-${segment.id}`;
  assert.ok(!edit.gameplay.jumpZones.some((entry) => entry.id === zone.id));
  edit.gameplay.jumpZones.push(zone);
  segment.edge.zone = zone.id;
  const plane = heightPlane(
    surface.polygon.map((point, index) => [...point, surface.height[index]]),
  );
  let center = segment.edge.a.slice(0, 2).map((value, axis) => (value + segment.edge.b[axis]) / 2);
  if (!pointInGameplayPolygon(center, surface.polygon, true)) {
    // Move from the closest boundary toward this slope's interior; this stage
    // only proposes an authoring candidate, never silently repairs an export.
    const centroid = [0, 1].map(
      (axis) =>
        surface.polygon.reduce((sum, point) => sum + point[axis], 0) / surface.polygon.length,
    );
    const candidates = surface.polygon.map((a, index) => {
      const b = surface.polygon[(index + 1) % surface.polygon.length];
      const delta = b.map((value, axis) => value - a[axis]);
      const t = Math.max(
        0,
        Math.min(
          1,
          delta.reduce((sum, value, axis) => sum + value * (center[axis] - a[axis]), 0) /
            delta.reduce((sum, value) => sum + value * value, 0),
        ),
      );
      return a.map((value, axis) => value + t * delta[axis]);
    });
    candidates.sort(
      (a, b) =>
        Math.hypot(...a.map((v, i) => v - center[i])) -
        Math.hypot(...b.map((v, i) => v - center[i])),
    );
    center = candidates[0].map((value, axis) => value + 0.35 * (centroid[axis] - value));
  }
  if (asset === "york-central-south-red-timber-house" && redSlide) {
    const ridge = surface.polygon[2].map((value, axis) => value - surface.polygon[1][axis]);
    const length = Math.hypot(...ridge);
    assert.ok(length > 0);
    center = center.map((value, axis) => value + (redSlide * ridge[axis]) / length);
    assert.ok(pointInGameplayPolygon(center, surface.polygon, true));
  }
  if (interiorFraction) {
    const centroid = [0, 1].map(
      (axis) =>
        surface.polygon.reduce((sum, point) => sum + point[axis], 0) / surface.polygon.length,
    );
    center = center.map((value, axis) => value + interiorFraction * (centroid[axis] - value));
    assert.ok(pointInGameplayPolygon(center, surface.polygon, true));
  }
  zone.polygon = surface.polygon.map((point, index) => [...point, surface.height[index]]);
  zone.anchor = [...center, planeHeight(plane, center)];
  zone.node = surface.node;
  sides.push({
    edit,
    descriptor: JSON.parse(bytes),
    plane,
    center,
    surface,
    segment,
    before,
    zone,
  });
}
const [first, second] = sides;
let tangent = [first.plane[1] - second.plane[1], second.plane[0] - first.plane[0]];
const length = Math.hypot(...tangent);
assert.ok(length > 1e-6);
tangent = tangent.map((value) => value / length);
const old = first.segment.edge.b.map((value, axis) => value - first.segment.edge.a[axis]);
if (tangent[0] * old[0] + tangent[1] * old[1] < 0) tangent = tangent.map((value) => -value);
for (const [index, side] of sides.entries()) {
  for (const [key, sign] of [
    ["a", -1],
    ["b", 1],
  ]) {
    const point = side.center.map(
      (value, axis) => value + (tangent[axis] * sign * (index ? -1 : 1) * span) / 2,
    );
    assert.ok(pointInGameplayPolygon(point, side.surface.polygon, true));
    side.segment.edge[key] = [...point, planeHeight(side.plane, point)];
  }
  validateAssetGameplay(side.edit.gameplay, side.descriptor);
}
const vectors = sides.map(({ segment: { edge } }) =>
  edge.b.map((value, axis) => value - edge.a[axis]),
);
assert.ok(vectors[0].every((value, axis) => Math.abs(value + vectors[1][axis]) < 1e-6));
const output = await fs.mkdtemp("work/needs-review/map-compile/york-roof-facing-receiver-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(`${output}/scene.json`, JSON.stringify(scene));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({
    source,
    redSlide,
    span,
    interiorFraction,
    published: false,
    scope: "alternate-receiving-slope-requires-placement-flight-and-mesh-review",
    sides: sides.map(({ edit, surface, before, segment, zone }) => ({
      asset: edit.asset,
      surface: surface.id,
      before,
      after: { segment, zone },
    })),
  }),
);
console.log(output);
