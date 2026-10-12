import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";

// Asset-local authoring repairs. Publishing remains guarded by the reviewed
// descriptor/model digests; the map compiler does not need this recipe.
const reviewed = {
  "croisement01-south-cut-stump": {
    descriptor: "05c5f3b859e04e20682e6b6f0d3ba0681e3fab7e1c630664cae47bf2536785e4",
    model: "6ce5fc545e76ffd6d87de3fd82140287b442812aab955594b2e709cd26ccfe2e",
    repair(gameplay) {
      const segment = gameplay.jumpSegments.find((item) => item.id === "jump-pair-5-side-0");
      assert.ok(segment);
      const before = structuredClone(segment);
      const [a, b] = [segment.edge.a, segment.edge.b];
      assert.ok(b[1] - b[2] - a[1] + a[2] - (b[0] - a[0]) > 0);
      for (const point of [a, b]) {
        point[0] += 1;
        point[1] += 1;
      }
      gameplay.draft.issues.push(
        "Cross-asset stump takeoff moved inward by one local X/Y unit so its midpoint supports the walking approach. Rotated click-region visibility and flight contacts remain under review.",
      );
      return { before, after: segment };
    },
  },
  "york-bridge-square-scaffolded-corner-house": {
    descriptor: "0eed6df64a0cdf305108d9bdd2ef588af913fdc3a6bce117d44095c3fa8bd0fb",
    model: "3e9a64b7556c5ccb0e4c05cfd8866f186d2d86249b7448aab75db0bec555fdeb",
    repair(gameplay) {
      const surface = gameplay.surfaces.find((item) => item.id === "building-138-walk-0");
      const pair = gameplay.jumpPairs.find((item) => item.id === "jump-pair-49");
      assert.ok(surface && pair && Array.isArray(surface.height));
      const points = surface.polygon.map(([x, y], i) => [
        x,
        y - surface.height[i],
        surface.height[i],
      ]);
      const plane = heightPlane(points);
      const edge = pair.edges[0];
      const midpoint = edge.a.map((n, i) => (n + edge.b[i]) / 2);
      const projected = [midpoint[0], midpoint[1] - midpoint[2]];
      assert.ok(pointInGameplayPolygon(projected, points, true));
      const height = planeHeight(plane, projected);
      assert.ok(Math.abs(height - midpoint[2]) < 1.1);
      const zone = gameplay.jumpZones.find((item) => item.id === edge.zone);
      assert.ok(zone);
      const before = structuredClone(zone);
      zone.anchor = [projected[0], projected[1] + height, height];
      gameplay.draft.issues.push(
        "Roof jump receiving anchor follows the ledge face instead of the adjoining roof face. Narrow rotated navigation regions and broad click-region coverage remain under review.",
      );
      return { before, after: zone, surface: surface.id };
    },
  },
};
const ids = process.argv.slice(2);
assert.ok(
  ids.length && ids.every((id) => Object.hasOwn(reviewed, id)),
  "Supply reviewed asset IDs",
);
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8"));
const edits = [],
  reviews = [];
for (const id of ids) {
  const entry = index.assets.find((item) => item.id === id);
  assert.ok(entry);
  const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
  assert.equal(hash(bytes), reviewed[id].descriptor);
  assert.equal(entry.descriptor_sha256, hash(bytes));
  const modelSha256 = hash(await fs.readFile(`library/3d-assets/${entry.model}`));
  assert.equal(modelSha256, reviewed[id].model);
  const descriptor = JSON.parse(bytes);
  const gameplay = structuredClone(descriptor.gameplay);
  const review = reviewed[id].repair(gameplay);
  validateAssetGameplay(gameplay, descriptor);
  edits.push({ asset: id, descriptorSha256: hash(bytes), modelSha256, gameplay });
  reviews.push({ asset: id, ...review });
}
const output = await fs.mkdtemp("work/needs-review/map-compile/reviewed-jump-anchors-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(`${output}/review.json`, JSON.stringify(reviews));
console.log(output);
