import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readStoredMap } from "../pipeline/src/stored-map.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

// Unpublished ownership candidate: the terrace carries its top floor with it.
const [staged, compiledFile] = process.argv.slice(2);
assert.ok(staged && compiledFile, "Provide church stair edits and their Leicester descriptor");
const edits = JSON.parse(await fs.readFile(`${staged}/edits.json`, "utf8"));
assert.equal(edits.length, 1);
assert.equal(edits[0].asset, "leicester-church-side-tower");
edits[0].gameplay.draft ??= { issues: [] };
edits[0].gameplay.draft.issues.push(
  "Upper internal stair mesh lacks some treads; lower-entry character compositing remains unverified.",
);
const asset = "leicester-lower-bailey-terrace";
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
const entry = index.find((e) => e.id === asset);
const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
assert.equal(createHash("sha256").update(bytes).digest("hex"), entry.descriptor_sha256);
const descriptor = JSON.parse(bytes);
const gameplay = structuredClone(descriptor.gameplay);
assert.equal(gameplay.surfaces.length, 0);
const part = descriptor.parts.find((p) => p.node === "building-123");
const points = part.obstacle_local_game.points;
const height = points[0].z_top;
assert.ok(points.every((p) => Math.abs(p.z_top - height) < 1e-6));
const floor = {
  id: "building-123-owned-top",
  node: part.node,
  polygon: points.map((p) => [p.x, p.y]),
  height,
  holes: [],
  preserveMovementPrecision: true,
};
const scene = await readStoredMap("library/scenes/leicester.rhlos-map.json", "library");
const group = scene.groups.find((g) => g.id === asset);
assert.equal(group.transform.rot_deg, 0);
const { dx, dy, dz } = group.transform;
const compiled = JSON.parse(await fs.readFile(compiledFile, "utf8")).asset_geometry;
const lift = compiled.lifts.find((l) =>
  l.physical_navigation?.doors.some(
    (d) =>
      Math.hypot(d.outside[0] - 1553, d.outside[1] - 958.001003, d.outside[2] - 50.001003) < 1e-4,
  ),
);
assert.ok(lift);
const positions = floor.polygon.flatMap((p, i) =>
  Math.hypot(p[0] + dx - 1542, p[1] + dy - 945) < 0.01 ? [i] : [],
);
assert.equal(positions.length, 1);
positions.push((positions[0] + 1) % floor.polygon.length);
const before = positions.map((i) => floor.polygon[i]);
assert.ok(Math.hypot(before[1][0] + dx - 1613, before[1][1] + dy - 966) < 0.01);
const [a, b, c] = lift.physical_navigation.plane;
const localC = c + a * dx + b * dy - dz;
const after = before.map(([x, y]) => {
  const t = (a * x + b * y + localC - height) / (a * a + b * b);
  assert.ok(Math.abs(t) * Math.hypot(a, b) < 1, "Seam requires broader geometry review");
  return [x - t * a, y - t * b];
});
positions.forEach((i, j) => {
  floor.polygon[i] = after[j];
});
gameplay.surfaces.push(floor);
gameplay.projectionReceivers = [];
gameplay.movementClearances = [
  ...(gameplay.movementClearances ?? []),
  {
    ...structuredClone(floor),
    id: "building-123-owned-top-clearance",
  },
];
validateAssetGameplay(gameplay, descriptor);
edits.push({ asset, descriptorSha256: entry.descriptor_sha256, gameplay });
const output = await fs.mkdtemp("work/map-compile/church-terrace-floor-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify(
    {
      scope: "unpublished terrace-owned floor; mesh and integration review required",
      asset,
      compiledFile,
      height,
      before,
      after,
      beforeWorld: before.map(([x, y]) => [x + dx, y + dy, height + dz]),
      afterWorld: after.map(([x, y]) => [x + dx, y + dy, height + dz]),
    },
    null,
    2,
  ),
);
console.log(output);
