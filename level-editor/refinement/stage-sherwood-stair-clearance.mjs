// Stage a clearance experiment; native traversal and receiving-contact review are required.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { clipHeight, heightPlane } from "../shared/src/gameplay-plane.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const clipping = createRequire(new URL("../shared/package.json", import.meta.url))(
  "polygon-clipping",
);

const scene = await readStoredMap("library/scenes/sherwood.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", scene.assetSources, scene.sceneAssets);
const id = "sherwood-central-oak-platform";
const asset = assets.get(id);
const source = scene.assetSources.find((entry) => entry.id === id);
const stored = JSON.parse(await fs.readFile("library/scenes/sherwood.rhlos-map.json"));
const placement = stored.placements.find((entry) => entry.id === id);
assert.ok(asset && source && placement);
assert.equal(placement.transform.rot_deg, 0);
const imported = JSON.parse(await fs.readFile("library/game-data/Data/Levels/Sherwood.rhp.json"));
const areas = new Map();
let index = 0;
for (const layer of imported.motion_data.layers)
  for (const area of layer) {
    areas.set(index, area);
    index += 1 + area.obstacles.length;
  }
const area = areas.get(37);
assert.ok(area?.is_lift && area.obstacles.length === 0);
const gameplay = structuredClone(asset.gameplay);
const surface = gameplay.surfaces.find((entry) => entry.id === "gameplay-volume-98-walk-0");
assert.ok(surface?.projectionVolume && Array.isArray(surface.height));
const before = structuredClone(surface);
const [a, b, c] = heightPlane(surface.polygon.map(([x, y], i) => [x, y, surface.height[i]]));
assert.ok(Math.abs(1 - b) > 1e-6);
const points = area.polygon.points.map(([x, y]) => {
  x -= placement.transform.dx;
  y += placement.transform.dz - placement.transform.dy;
  const z = (a * x + b * y + c) / (1 - b);
  return [x, y + z, z];
});
const low = Math.min(...surface.height);
const high = Math.max(...surface.height);
const clearance = clipHeight(
  clipHeight(
    points.map(([x, y]) => [x, y]),
    [a, b, c - low],
  ),
  [-a, -b, high - c],
);
const regions = clipping.union([surface.polygon], [clearance]);
assert.equal(regions.length, 1);
assert.equal(regions[0].length, 1);
surface.polygon = regions[0][0].slice(0, -1);
if (process.argv.includes("--convex")) {
  const points = surface.polygon.toSorted((a, b) => a[0] - b[0] || a[1] - b[1]);
  const turn = (a, b, c) => (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
  const half = (points) => {
    const result = [];
    for (const point of points) {
      while (result.length > 1 && turn(result.at(-2), result.at(-1), point) <= 0) result.pop();
      result.push(point);
    }
    return result.slice(0, -1);
  };
  surface.polygon = [...half(points), ...half(points.toReversed())];
}
surface.height = surface.polygon.map(([x, y]) => a * x + b * y + c);
// The authored stair opening must cover its revised walkable footprint too.
// This clearance is owner-local and cannot remove another asset's collision.
const clearanceId = "central-oak-stair-movement-clearance";
gameplay.movementClearances = [
  ...(gameplay.movementClearances ?? []).filter((entry) => entry.id !== clearanceId),
  {
    id: clearanceId,
    node: surface.node,
    polygon: structuredClone(surface.polygon),
    height: [...surface.height],
    holes: [],
  },
];
validateAssetGameplay(gameplay, asset);
const output = await fs.mkdtemp("work/needs-review/map-compile/sherwood-stair-clearance-");
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
    scope: "staged-local-movement-clearance-not-traversal-or-render-proof",
    source,
    convexDiagnostic: process.argv.includes("--convex"),
    before,
    after: surface,
  }),
);
console.log(output);
