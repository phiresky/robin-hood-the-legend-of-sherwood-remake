import fs from "node:fs/promises";
import assert from "node:assert/strict";

const [stage] = process.argv.slice(2);
assert.ok(stage, "Provide the physical watermill platform stage");
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
assert.equal(edits.length, 1);
const { gameplay } = edits[0];
assert.equal(edits[0].asset, "leicester-watermill");
const surface = gameplay.surfaces.find((surface) => surface.id === "watermill-platform-floor");
assert.ok(surface);
assert.equal(gameplay.jumpPairs.length, 1);
const pair = gameplay.jumpPairs[0];
assert.equal(pair.id, "jump-pair-21");
const upperZone = gameplay.jumpZones.find((zone) => zone.id === pair.edges[0].zone);
assert.ok(upperZone);
upperZone.polygon = surface.polygon.map(([x, y]) => [x, y, surface.height]);
// The outward-facing platform end supplies a reusable geometric ledge. The
// ground counterpart stays asset-local and still requires a receiving surface.
const a = [...surface.polygon[0], surface.height];
const b = [...surface.polygon.at(-1), surface.height];
const dx = b[0] - a[0],
  dy = b[1] - a[1];
const length = Math.hypot(dx, dy);
// Keep rounded launch points on the receiving floor, inside its boundary.
for (const point of [a, b]) {
  point[0] += dy / length;
  point[1] -= dx / length;
}
// Avoid the adjacent boundary at each platform corner after grid rounding.
for (const [point, sign] of [
  [a, 1],
  [b, -1],
]) {
  point[0] += (sign * 2 * dx) / length;
  point[1] += (sign * 2 * dy) / length;
}
const groundHeight = pair.edges[1].a[2];
const ground = (point) => [
  point[0] - (40 * dy) / length,
  point[1] + (40 * dx) / length - surface.height + groundHeight,
  groundHeight,
];
const groundA = ground(b),
  groundB = ground(a);
for (const [axis, delta] of [
  [0, dx],
  [1, dy],
]) {
  groundA[axis] += (40 * delta) / length;
  groundB[axis] -= (40 * delta) / length;
}
const edges = [
  { zone: pair.edges[0].zone, a, b },
  { zone: pair.edges[1].zone, a: groundA, b: groundB },
];
gameplay.jumpPairs = [];
gameplay.jumpSegments = edges.map((edge, i) => ({
  id: `watermill-boundary-jump-${i}`,
  node: pair.node,
  long: true,
  edge,
  attachment: { maxGap: 100, maxRise: 20, maxDrop: 20, minOverlap: 2 },
}));
gameplay.draft.issues.push(
  "Unpublished boundary jump candidate: physical ledges replace the inset authored pair; native flight, approach, landing and rendered contact require review.",
);
const output = await fs.mkdtemp("work/map-compile/watermill-boundary-jump-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(`${output}/review.json`, JSON.stringify({ source: stage, before: pair, edges }));
console.log(output);
