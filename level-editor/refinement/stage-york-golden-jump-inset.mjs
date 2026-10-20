// Review an asset-local takeoff inset against rotated native landing probes.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const distance = Number(process.argv[2] ?? 1);
assert.ok(Number.isFinite(distance) && distance > 0 && distance <= 15);
const scene = JSON.parse(await fs.readFile("library/scenes/york.rhlos-map.json"));
const id = "york-central-south-golden-timber-house";
const ref = scene.assetSources.find((source) => source.id === id);
assert.ok(ref);
const bytes = await fs.readFile(`library/${ref.descriptor}`);
assert.equal(createHash("sha256").update(bytes).digest("hex"), ref.descriptor_sha256);
const asset = JSON.parse(bytes);
const gameplay = structuredClone(asset.gameplay);
const surface = gameplay.surfaces.find((surface) => surface.id === "building-341-walk-0");
const segment = gameplay.jumpSegments.find((segment) => segment.id === "jump-pair-42-side-0");
assert.ok(surface && segment && Array.isArray(surface.height));
const plane = heightPlane(surface.polygon.map((point, i) => [...point, surface.height[i]]));
const center = [0, 1].map(
  (axis) => surface.polygon.reduce((sum, point) => sum + point[axis], 0) / surface.polygon.length,
);
assert.ok(pointInGameplayPolygon(center, surface.polygon));
const midpoint = [0, 1].map((axis) => (segment.edge.a[axis] + segment.edge.b[axis]) / 2);
const delta = center.map((value, axis) => value - midpoint[axis]);
const length = Math.hypot(...delta);
assert.ok(length > distance);
const offset = delta.map((value) => (value * distance) / length);
const before = structuredClone(segment);
for (const key of ["a", "b"]) {
  const old = segment.edge[key];
  const point = [old[0] + offset[0], old[1] + offset[1]];
  assert.ok(pointInGameplayPolygon(point, surface.polygon));
  segment.edge[key] = [...point, old[2] + planeHeight(plane, point) - planeHeight(plane, old)];
}
validateAssetGameplay(gameplay, asset);
const output = await fs.mkdtemp("work/needs-review/map-compile/york-golden-jump-inset-");
await fs.writeFile(
  `${output}/edits.json`,
  JSON.stringify([
    {
      asset: id,
      descriptorSha256: ref.descriptor_sha256,
      modelSha256: ref.model_sha256,
      gameplay,
    },
  ]),
);
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({
    scope: "unpublished-inset-candidate-requires-flight-and-native-review",
    distance,
    offset,
    before,
    after: segment,
  }),
);
console.log(output);
