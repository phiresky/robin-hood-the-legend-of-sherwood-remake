// Keep the takeoff on its authored roof plane while reviewing approach clearance.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { readStoredMap } from "../pipeline/src/stored-map.ts";

const distance = Number(process.argv[2] ?? 3);
assert.ok(Number.isFinite(distance) && distance > 0 && distance <= 8);
const id = "york-central-south-red-timber-house";
const scene = JSON.parse(await fs.readFile("library/scenes/york.rhlos-map.json"));
const source = scene.assetSources.find((ref) => ref.id === id);
assert.ok(source);
const bytes = await fs.readFile(`library/${source.descriptor}`);
assert.equal(createHash("sha256").update(bytes).digest("hex"), source.descriptor_sha256);
const descriptor = JSON.parse(bytes);
const gameplay = structuredClone(descriptor.gameplay);
const surface = gameplay.surfaces.find((s) => s.id === "building-323-walk-0");
const segment = gameplay.jumpSegments.find((s) => s.id === "jump-pair-45-side-0");
assert.ok(surface && segment && Array.isArray(surface.height));
const plane = heightPlane(surface.polygon.map((point, i) => [...point, surface.height[i]]));
const ridge = surface.polygon[2].map((n, i) => n - surface.polygon[1][i]);
const length = Math.hypot(...ridge);
const inward = [-ridge[1] / length, ridge[0] / length];
const before = structuredClone(segment);
const zone = gameplay.jumpZones.find((z) => z.id === segment.edge.zone);
assert.ok(zone);
const zoneBefore = structuredClone(zone);
if (process.argv.includes("--surface-zone")) {
  assert.ok(pointInGameplayPolygon(zone.anchor, surface.polygon, true));
  assert.ok(Math.abs(planeHeight(plane, zone.anchor) - zone.anchor[2]) < 0.01);
  zone.polygon = surface.polygon.map((point, i) => [...point, surface.height[i]]);
}
for (const point of [segment.edge.a, segment.edge.b]) {
  point[0] += inward[0] * distance;
  point[1] += inward[1] * distance;
  assert.ok(pointInGameplayPolygon(point, surface.polygon, true));
  point[2] = planeHeight(plane, point);
}
validateAssetGameplay(gameplay, descriptor);
const output = await fs.mkdtemp("work/needs-review/map-compile/york-red-roof-inset-");
await fs.writeFile(
  `${output}/scene.json`,
  JSON.stringify(await readStoredMap("library/scenes/york.rhlos-map.json", "library")),
);
await fs.writeFile(
  `${output}/edits.json`,
  JSON.stringify([
    {
      asset: id,
      descriptorSha256: source.descriptor_sha256,
      modelSha256: source.model_sha256,
      gameplay,
    },
  ]),
);
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({
    scope: "unpublished-takeoff-inset-requires-placement-flight-and-mesh-review",
    distance,
    inward,
    plane,
    before,
    after: segment,
    zoneBefore,
    zoneAfter: zone,
  }),
);
console.log(output);
