import fs from "node:fs/promises";
import assert from "node:assert/strict";

const [stage] = process.argv.slice(2);
assert.ok(stage, "Provide the boundary jump review stage");
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
assert.equal(edits.length, 1);
assert.equal(edits[0].asset, "leicester-watermill");
const { gameplay } = edits[0];
assert.equal(gameplay.jumpSegments.length, 2);
const [upper, lower] = gameplay.jumpSegments;
assert.equal(upper.id, "watermill-boundary-jump-0");
const before = structuredClone(gameplay.jumpSegments);
const a = upper.edge.a,
  b = upper.edge.b;
const dx = b[0] - a[0],
  dy = b[1] - a[1],
  length = Math.hypot(dx, dy);
// Review a standing-footprint inset and shorten the span at the corners.
// The receiving surface and solid platform volume are unchanged.
for (const [point, sign] of [
  [a, 1],
  [b, -1],
]) {
  point[0] += (5 * dy + sign * 4 * dx) / length;
  point[1] += (-5 * dx + sign * 4 * dy) / length;
}
// The wider corner needs additional room for the inward walking continuation.
a[0] += (5 * dx) / length;
a[1] += (5 * dy) / length;
const center = lower.edge.a.map((value, axis) => (value + lower.edge.b[axis]) / 2);
lower.edge.a = center.map((value, axis) => value + (b[axis] - a[axis]) / 2);
lower.edge.b = center.map((value, axis) => value - (b[axis] - a[axis]) / 2);
gameplay.jumpPairs = [
  {
    id: "watermill-walking-jump",
    node: upper.node,
    long: true,
    edges: [upper.edge, lower.edge],
  },
];
gameplay.jumpSegments = [];
gameplay.draft.issues.push(
  "Unpublished walking jump candidate: inset launch and landing span requires full walking review. Authored connection retains flight-clearance warnings, including possible combat animation intersection with the platform.",
);
const output = await fs.mkdtemp("work/map-compile/watermill-walking-jump-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({ source: stage, before, after: gameplay.jumpPairs }),
);
console.log(output);
