// Author a supported approach apron from the climb's existing lower landing.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { fixedPolygonBoolean } from "../shared/src/fixed-polygon-boolean.ts";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { clipHeight, heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const document = await readStoredMap("library/scenes/york.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
const id = "york-east-riverside-northern-wall-stair";
const asset = assets.get(id);
assert.ok(asset);
const gameplay = structuredClone(asset.gameplay);
const flight = gameplay.surfaces.find((surface) => surface.id === "building-286-walk-0");
const landing = gameplay.surfaces.find((surface) => surface.id === "building-287-walk-0");
const lift = gameplay.lifts.find((lift) => lift.surface === flight.id);
assert.ok(flight && landing && lift && !lift.joins);
const plane = heightPlane(landing.polygon.map(([x, y], i) => [x, y, landing.height[i]]));
const lowDoor = lift.doors.find((door) => door.type === 5);
assert.ok(lowDoor);
const seam = lowDoor.middle.slice(0, 2);
const edge = flight.polygon.at(-1).map((value, i) => value - flight.polygon.at(-2)[i]);
const length = Math.hypot(...edge);
let normal = [-edge[1] / length, edge[0] / length];
if (normal.reduce((sum, value, i) => sum + value * (lowDoor.outside[i] - seam[i]), 0) < 0)
  normal = normal.map((value) => -value);
const distance = 24;
const limit = normal[0] * seam[0] + normal[1] * seam[1] + distance;
const before = structuredClone(landing);
const apron = clipHeight(landing.polygon, [-normal[0], -normal[1], limit]);
const remainder = clipHeight(landing.polygon, [normal[0], normal[1], -limit]);
assert.ok(apron.length >= 3 && remainder.length >= 3);
const partitionResidual = fixedPolygonBoolean("xor", [before.polygon], [[apron], [remainder]])
  .flat()
  .reduce(
    (total, ring) =>
      total +
      Math.abs(
        ring.reduce((sum, p, i) => {
          const q = ring[(i + 1) % ring.length];
          return sum + p[0] * q[1] - q[0] * p[1];
        }, 0),
      ) /
        2,
    0,
  );
assert.ok(partitionResidual < 0.0001, `Partition exceeds clipping precision: ${partitionResidual}`);
const onLanding = (point) => [...point, planeHeight(plane, point)];
const anchor = (offset) => onLanding(seam.map((value, i) => value + normal[i] * offset));
const apronId = `${landing.id}-climb-apron`;
gameplay.surfaces.push({
  ...structuredClone(landing),
  id: apronId,
  polygon: apron,
  height: apron.map((point) => planeHeight(plane, point)),
});
landing.polygon = remainder;
landing.height = remainder.map((point) => planeHeight(plane, point));
const door = {
  ...lowDoor,
  node: landing.node,
  middle: anchor(distance),
  inside: anchor(distance - 10),
  outside: anchor(distance + 14),
};
const join = onLanding(seam);
assert.ok(Math.abs(join[2] - lowDoor.middle[2]) < 1e-4);
lift.doors = lift.doors.filter((door) => door !== lowDoor);
lift.joins = [join];
gameplay.lifts.push({
  id: `${lift.id}-apron`,
  node: landing.node,
  surface: apronId,
  type: lift.type,
  direction: [...lift.direction],
  joins: [join],
  doors: [door],
});
validateAssetGameplay(gameplay, asset);
const ref = document.assetSources.find((ref) => ref.id === id);
const output = await fs.mkdtemp("work/needs-review/map-compile/york-wall-animation-apron-");
await fs.writeFile(
  `${output}/edits.json`,
  JSON.stringify([
    {
      asset: id,
      descriptorSha256: ref.descriptor_sha256,
      modelSha256: ref.model_sha256,
      gameplay,
    },
  ]),
);
await fs.writeFile(`${output}/scene.json`, JSON.stringify(document));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({
    scope: "unpublished-animation-apron-investigation",
    before,
    apron,
    remainder,
    beforeDoor: lowDoor,
    afterDoor: door,
    distance,
    partitionResidual,
  }),
);
console.log(output);
