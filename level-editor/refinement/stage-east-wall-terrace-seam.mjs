import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { heightPlane } from "../shared/src/gameplay-plane.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

// Stage the receiving asset's edge independently of the stair definition.
// Mesh review and placed actor checks are required before publication.
const [staged] = process.argv.slice(2);
assert.ok(staged, "Provide the reviewed turret stair candidate");
const edits = JSON.parse(await fs.readFile(`${staged}/edits.json`, "utf8"));
assert.equal(edits.length, 1);
assert.equal(edits[0].asset, "leicester-east-wall-turret");
edits[0].gameplay.draft ??= { issues: [] };
edits[0].gameplay.draft.issues.push(
  "Subpixel landing mesh discrepancies remain; rendered character integration is unverified.",
);
const asset = "leicester-lower-bailey-terrace";
const scene = await readStoredMap("library/scenes/leicester.rhlos-map.json", "library");
const sources = scene.assetSources.filter((s) => [asset, edits[0].asset].includes(s.id));
const descriptors = await pinnedDescriptors("library", sources, []);
assert.equal(
  sources.find((s) => s.id === edits[0].asset).descriptor_sha256,
  edits[0].descriptorSha256,
);
const descriptor = descriptors.get(asset);
const gameplay = structuredClone(descriptor.gameplay);
const stairGroup = scene.groups.find((g) => g.id === edits[0].asset);
const group = scene.groups.find((g) => g.id === asset);
assert.equal(stairGroup.transform.rot_deg, 0);
assert.equal(group.transform.rot_deg, 0);
const floor = edits[0].gameplay.surfaces.find((s) => s.id === "building-125-walk-0");
const plane = heightPlane(floor.polygon.map(([x, y], i) => [x, y, floor.height[i]]));
const [a, b] = plane;
const c =
  plane[2] +
  stairGroup.transform.dz -
  group.transform.dz +
  a * (group.transform.dx - stairGroup.transform.dx) +
  b * (group.transform.dy - stairGroup.transform.dy);
const landing = gameplay.surfaces.find((s) => s.id === "building-123-owned-top");
assert.ok(landing && typeof landing.height === "number");
const height = landing.height;
const { dx, dy } = group.transform;
const start = landing.polygon.findIndex(
  ([x, y]) => Math.hypot(x + dx - 1624, y + dy - 1216) < 0.01,
);
assert.ok(start >= 0);
const end = (start + 1) % landing.polygon.length;
assert.ok(
  Math.hypot(landing.polygon[end][0] + dx - 1670, landing.polygon[end][1] + dy - 1253) < 0.01,
);
const before = [landing.polygon[start], landing.polygon[end]];
const after = before.map(([x, y]) => {
  const t = (a * x + b * y + c - height) / (a * a + b * b);
  assert.ok(Math.abs(t) * Math.hypot(a, b) < 1, "Receiving edge needs broader review");
  return [x - a * t, y - b * t];
});
landing.polygon[start] = after[0];
landing.polygon[end] = after[1];
const clearance = gameplay.movementClearances.find(
  (s) => s.id === "building-123-owned-top-clearance",
);
assert.ok(clearance);
clearance.polygon = structuredClone(landing.polygon);
gameplay.draft ??= { issues: [] };
gameplay.draft.issues.push(
  "Stair receiving seams have subpixel mesh discrepancies; rendered character integration is unverified.",
);
validateAssetGameplay(gameplay, descriptor);
const source = sources.find((s) => s.id === asset);
const bytes = await fs.readFile(`library/${source.descriptor}`);
assert.equal(createHash("sha256").update(bytes).digest("hex"), source.descriptor_sha256);
edits.push({ asset, descriptorSha256: source.descriptor_sha256, gameplay });
const output = await fs.mkdtemp("work/map-compile/east-wall-terrace-seam-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify(
    {
      scope: "unpublished terrace receiving seam; mesh and integration review required",
      asset,
      height,
      before,
      after,
      beforeWorld: before.map(([x, y]) => [x + dx, y + dy, height + group.transform.dz]),
      afterWorld: after.map(([x, y]) => [x + dx, y + dy, height + group.transform.dz]),
    },
    null,
    2,
  ),
);
console.log(output);
