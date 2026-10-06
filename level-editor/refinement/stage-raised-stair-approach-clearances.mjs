import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { fixedPolygonBoolean } from "../shared/src/fixed-polygon-boolean.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh } from "../pipeline/src/mask-recovery-mesh.ts";
import { sceneToGame, gltfToScene } from "../shared/src/geometry.ts";

// Derive a raised approach opening from the mesh, restricted to the stair
// contact strip. This is an authoring candidate, never an export fallback.
const [source] = process.argv.slice(2);
assert.ok(source);
const edits = JSON.parse(await fs.readFile(`${source}/edits.json`, "utf8"));
assert.equal(edits.length, 1);
const edit = edits[0];
assert.equal(edit.asset, "york-east-riverside-southern-wall-stair");
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
const entry = index.find((e) => e.id === edit.asset);
const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
assert.equal(createHash("sha256").update(bytes).digest("hex"), edit.descriptorSha256);
const descriptor = JSON.parse(bytes),
  gameplay = edit.gameplay;
const modelSha256 = createHash("sha256")
  .update(await fs.readFile(`library/3d-assets/${entry.model}`))
  .digest("hex");
const model = await loadSceneModel("library", {
  id: edit.asset,
  role: "objects",
  model: `3d-assets/${entry.model}`,
  model_sha256: modelSha256,
  descriptor: `3d-assets/${entry.descriptor}`,
  descriptor_sha256: edit.descriptorSha256,
  resources: descriptor.resources ?? [],
});
const lift = gameplay.lifts.find((l) => l.id === "building-268-lift");
const door = lift.doors.find((d) => d.type === 5);
const surface = gameplay.surfaces.find((s) => s.id === lift.surface);
const plane = heightPlane(surface.polygon.map((p, i) => [...p, surface.height[i]]));
const z = door.outside[2];
for (const floor of gameplay.surfaces) {
  if (floor.id !== lift.surface) continue;
  const receiver = floor.projectionMaterials;
  if (!receiver) continue;
  receiver.footprint = floor.polygon.map((p, i) => [...p, floor.height[i]]);
  let best = 0;
  for (let i = 1; i < receiver.footprint.length - 1; i++) {
    const [a, b, c] = [receiver.footprint[0], receiver.footprint[i], receiver.footprint[i + 1]];
    const area = Math.abs((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]));
    if (area > best) {
      best = area;
      receiver.planePoints = [a, b, c];
    }
  }
  assert.ok(best > 1e-6);
  heightPlane(receiver.planePoints);
  receiver.priorityHeight = Math.max(...floor.height);
}
const seam = surface.polygon.filter((p) => Math.abs(planeHeight(plane, p) - z) < 1e-5);
assert.equal(seam.length, 2);
const length = Math.hypot(plane[0], plane[1]);
const sign = Math.sign(planeHeight(plane, door.outside) - z);
const offset = plane.slice(0, 2).map((v) => (v * sign * 30) / length);
const strip = [seam[0], seam[1], ...[seam[1], seam[0]].map((p) => p.map((v, i) => v + offset[i]))];
const added = [];
for (const node of ["building-268", "building-282--component-wall-stair", "building-285"]) {
  const flight = {
    id: `${node}-reviewed-stair-flight`,
    node,
    polygon: structuredClone(surface.polygon),
    height: [...surface.height],
    holes: [],
    preserveMovementPrecision: true,
  };
  gameplay.movementClearances.push(flight);
  added.push(flight);
  const landing = gameplay.surfaces.find(
    (s) => s.id === "building-282--component-wall-stair-walk-0",
  );
  assert.ok(landing && !landing.holes?.length);
  const upper = {
    id: `${node}-reviewed-upper-landing`,
    node,
    polygon: structuredClone(landing.polygon),
    height: [...landing.height],
    holes: [],
    preserveMovementPrecision: true,
  };
  gameplay.movementClearances.push(upper);
  added.push(upper);
  const triangles = maskRecoveryMesh(model, node, (p) =>
    sceneToGame({ kind: "oblique-orthographic", elevation_deg: 35 }, gltfToScene(p)),
  );
  const blockers = triangles.flatMap((triangle) => {
    const clipped = [];
    for (let i = 0; i < 3; i++) {
      const a = triangle[i],
        b = triangle[(i + 1) % 3];
      const da = a[2] - z - 0.1,
        db = b[2] - z - 0.1;
      if (da >= 0) clipped.push(a.slice(0, 2));
      if (da >= 0 !== db >= 0) {
        const t = da / (da - db);
        clipped.push([a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]);
      }
    }
    return clipped.length >= 3 ? [[clipped]] : [];
  });
  const polygons = fixedPolygonBoolean("difference", [strip], blockers);
  assert.ok(polygons.length);
  for (const [i, polygon] of polygons.entries()) {
    const ring = polygon[0].slice(0, -1);
    const clearance = {
      id: `${node}-raised-approach-${i}`,
      node,
      polygon: ring,
      height: ring.map(() => z),
      holes: polygon.slice(1).map((r) => r.slice(0, -1)),
      preserveMovementPrecision: true,
    };
    gameplay.movementClearances.push(clearance);
    added.push(clearance);
  }
}
validateAssetGameplay(gameplay, descriptor);
const review = JSON.parse(await fs.readFile(`${source}/review.json`, "utf8"));
review.changes.push({
  raisedApproachClearances: added,
  modelSha256,
  meshHeightMargin: 0.1,
  requiresMeshAndNativeReview: true,
});
const output = await fs.mkdtemp("work/map-compile/raised-stair-approach-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(`${output}/review.json`, JSON.stringify(review));
console.log(JSON.stringify({ output, added: added.length }));
