import fs from "node:fs/promises";
import assert from "node:assert/strict";

// Review candidates only. Preserve floor, solids, zones, span length and terrain
// heights while varying the standing inset and ground-side separation.
const [stage, insetArgument, groundArgument = "0"] = process.argv.slice(2);
const inset = Number(insetArgument);
const groundOffset = Number(groundArgument);
assert.ok(stage && Number.isFinite(inset) && inset >= 0 && inset <= 6);
assert.ok(Number.isFinite(groundOffset) && groundOffset >= 0 && groundOffset <= 40);
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
assert.equal(edits.length, 1);
assert.equal(edits[0].asset, "leicester-watermill");
const { gameplay } = edits[0];
assert.equal(gameplay.jumpPairs.length, 1);
const pair = gameplay.jumpPairs[0];
assert.equal(pair.id, "watermill-walking-jump");
const before = structuredClone(pair);
const [upper, lower] = pair.edges;
const surface = gameplay.surfaces.find((s) => s.id === "watermill-platform-floor");
assert.ok(surface && typeof surface.height === "number");
const boundaryA = surface.polygon[0];
const boundaryB = surface.polygon.at(-1);
const dx = boundaryB[0] - boundaryA[0];
const dy = boundaryB[1] - boundaryA[1];
const length = Math.hypot(dx, dy);
const inward = [dy / length, -dx / length];
const currentInset =
  (upper.a[0] - boundaryA[0]) * inward[0] +
  (upper.a[1] - boundaryA[1]) * inward[1];
assert.ok(Math.abs(currentInset - 6) < 1e-6, "Expected the published six-unit inset");
for (const point of [upper.a, upper.b]) {
  point[0] += (inset - currentInset) * inward[0];
  point[1] += (inset - currentInset) * inward[1];
}
for (const point of [lower.a, lower.b]) {
  point[0] -= groundOffset * inward[0];
  point[1] -= groundOffset * inward[1];
}
gameplay.draft.issues.push(
  `Unpublished jump contact review: ${inset}-unit platform inset and ${groundOffset}-unit extra ground separation; zone, walking and flight validation required.`,
);
const output = await fs.mkdtemp("work/map-compile/watermill-jump-contact-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({ stage, inset, groundOffset, before, after: pair }),
);
console.log(output);
