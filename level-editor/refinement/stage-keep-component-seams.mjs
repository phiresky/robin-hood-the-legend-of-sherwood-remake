import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { translateGameplayFrames } from "../pipeline/src/translate-gameplay-frames.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

// Stage reviewed composite seam changes in the owning component frames. This
// reads assets only and deliberately leaves publication to the pinned installer.
const [baselineDirectory, candidateDirectory] = process.argv.slice(2);
assert.ok(baselineDirectory && candidateDirectory, "Provide baseline and candidate directories");
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const read = async (file) => {
  const bytes = await fs.readFile(file, "utf8");
  return { data: JSON.parse(bytes), sha256: hash(bytes) };
};
const baseline = await read(`${baselineDirectory}/candidate.gameplay.json`);
const candidate = await read(`${candidateDirectory}/candidate.gameplay.json`);
const index = (await read("library/3d-assets/index.json")).data.assets;
const load = async (id) => {
  const entry = index.find((entry) => entry.id === id);
  assert.ok(entry, `Missing asset: ${id}`);
  const value = await read(`library/3d-assets/${entry.descriptor}`);
  assert.equal(value.sha256, entry.descriptor_sha256, `Stale descriptor: ${id}`);
  return { entry, ...value };
};
const composite = await load("derby-great-keep");
const sources = await Promise.all(
  ["derby-keep-central-gallery", "derby-keep-west-tower"].map(load),
);
const changes = [];
const equalGeometry = (actual, expected, label) => {
  const visit = (a, b) => {
    if (typeof a === "number" && typeof b === "number")
      assert.ok(Math.abs(a - b) < 1e-5, `${label}: changed coordinate ${a} / ${b}`);
    else if (Array.isArray(a) && Array.isArray(b)) {
      assert.equal(a.length, b.length, label);
      a.forEach((value, i) => visit(value, b[i]));
    } else assert.deepEqual(a, b, label);
  };
  visit(actual, expected);
};
const offsets = new Map();
for (const source of sources) {
  const map = new Map();
  for (const part of source.data.parts) {
    const target = composite.data.parts.find((value) => value.node === part.node);
    assert.ok(target, `Missing composite part: ${part.node}`);
    const a = part.obstacle_local_game.points[0];
    const b = target.obstacle_local_game.points[0];
    map.set(part.node, [b.x - a.x, b.y - a.y, b.z_bottom - a.z_bottom]);
  }
  offsets.set(source.entry.id, map);
  source.translated = translateGameplayFrames(source.data.gameplay, map);
  source.next = structuredClone(source.data.gameplay);
}
const owner = (node) => {
  const matches = sources.filter((source) => offsets.get(source.entry.id).has(node));
  assert.equal(matches.length, 1, `Ambiguous component owner: ${node}`);
  return matches[0];
};
const localSurface = (surface, source) => {
  const delta = offsets
    .get(source.entry.id)
    .get(surface.node)
    .map((v) => -v);
  return translateGameplayFrames(
    { version: 1, collision: "none", surfaces: [surface], doors: [] },
    new Map([[surface.node, delta]]),
  ).surfaces[0];
};
for (const after of candidate.data.surfaces) {
  const before = baseline.data.surfaces.find((surface) => surface.id === after.id);
  assert.ok(before, `Unexpected new surface: ${after.id}`);
  if (JSON.stringify(before) === JSON.stringify(after)) continue;
  assert.deepEqual(
    {
      ...before,
      polygon: after.polygon,
      height: after.height,
      preserveMovementPrecision: after.preserveMovementPrecision,
    },
    after,
    `Unexpected non-seam surface edit: ${after.id}`,
  );
  const source = owner(after.node);
  const translated = source.translated.surfaces.find((surface) => surface.id === after.id);
  const current = source.next.surfaces.find((surface) => surface.id === after.id);
  assert.ok(translated && current, `Missing component surface: ${after.id}`);
  equalGeometry(translated.polygon, before.polygon, after.id);
  equalGeometry(translated.height, before.height, after.id);
  const local = localSurface(after, source);
  const previous = structuredClone(current);
  current.polygon = local.polygon;
  current.height = local.height;
  current.preserveMovementPrecision = local.preserveMovementPrecision;
  changes.push({ asset: source.entry.id, surface: after.id, before: previous, after: current });
}
for (const lift of candidate.data.lifts) {
  const beforeLift = baseline.data.lifts.find((value) => value.id === lift.id);
  assert.ok(beforeLift);
  const source = owner(lift.node);
  const translated = source.translated.lifts.find((value) => value.id === lift.id);
  const current = source.next.lifts.find((value) => value.id === lift.id);
  assert.ok(translated && current);
  for (const after of lift.doors) {
    const before = beforeLift.doors.find((door) => door.id === after.id);
    assert.ok(before);
    if (JSON.stringify(before.middle) === JSON.stringify(after.middle)) continue;
    assert.deepEqual(
      { ...before, middle: after.middle },
      after,
      `Unexpected door edit: ${after.id}`,
    );
    equalGeometry(
      translated.doors.find((door) => door.id === after.id).middle,
      before.middle,
      after.id,
    );
    const door = current.doors.find((door) => door.id === after.id);
    const previous = [...door.middle];
    const delta = offsets.get(source.entry.id).get(after.node);
    door.middle = after.middle.map((value, i) => value - delta[i]);
    changes.push({ asset: source.entry.id, door: after.id, before: previous, after: door.middle });
  }
}
for (const clearance of candidate.data.movementClearances) {
  const before = baseline.data.movementClearances.find((value) => value.id === clearance.id);
  if (before) {
    assert.deepEqual(clearance, before, `Unexpected existing clearance edit: ${clearance.id}`);
    continue;
  }
  const source = owner(clearance.node);
  const lift = candidate.data.lifts.find((value) => value.node === clearance.node);
  const floor = candidate.data.surfaces.find((surface) => surface.id === lift?.surface);
  assert.ok(floor, `New clearance has no stair floor: ${clearance.id}`);
  assert.deepEqual(clearance.polygon, floor.polygon);
  assert.deepEqual(clearance.height, floor.height);
  assert.deepEqual(clearance.holes ?? [], floor.holes ?? []);
  source.next.movementClearances ??= [];
  assert.ok(!source.next.movementClearances.some((value) => value.id === clearance.id));
  const local = localSurface(clearance, source);
  source.next.movementClearances.push(local);
  changes.push({ asset: source.entry.id, clearance: clearance.id, after: local });
}
assert.equal(changes.filter((change) => change.surface).length, 8);
assert.equal(changes.filter((change) => change.door).length, 7);
assert.equal(changes.filter((change) => change.clearance).length, 3);
const output = await fs.mkdtemp("work/map-compile/keep-component-seams-");
const edits = sources.map((source) => {
  validateAssetGameplay(source.next, source.data);
  return { asset: source.entry.id, descriptorSha256: source.sha256, gameplay: source.next };
});
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify(
    {
      baseline: { directory: baselineDirectory, sha256: baseline.sha256 },
      candidate: { directory: candidateDirectory, sha256: candidate.sha256 },
      composite: { asset: composite.entry.id, sha256: composite.sha256 },
      sources: sources.map((source) => ({ asset: source.entry.id, sha256: source.sha256 })),
      scope:
        "Unpublished component-local seam and clearance corrections; requires placement review",
      changes,
    },
    null,
    2,
  ),
);
console.log(
  JSON.stringify({ output, assets: edits.map((edit) => edit.asset), changes: changes.length }),
);
