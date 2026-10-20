// Restore the sloped lane's own walking definition from its retained receiver.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";

const asset = "york-west-town-raised-lane";
const scene = JSON.parse(await fs.readFile("library/scenes/york.rhlos-map.json"));
const source = scene.assetSources.find((entry) => entry.id === asset);
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const bytes = await fs.readFile(`library/${source.descriptor}`);
assert.equal(hash(bytes), source.descriptor_sha256);
assert.equal(hash(await fs.readFile(`library/${source.model}`)), source.model_sha256);
const descriptor = JSON.parse(bytes);
const gameplay = structuredClone(descriptor.gameplay);
const part = descriptor.parts.find((part) => part.node === "building-088");
assert.ok(part?.obstacle_local_game?.projection_area);
assert.ok(!gameplay.surfaces.some((surface) => surface.node === part.node));
const points = part.obstacle_local_game.points;
const minimum = Math.min(...points.map((point) => point.z_top));
const bottom = points.filter((point) => Math.abs(point.z_top - minimum) < 1e-7);
assert.equal(bottom.length, 2, "Lane needs its two shared bottom-edge vertices");
const upper = points.find((point) => point.z_top > minimum + 1);
assert.ok(upper);
const plane = heightPlane([...bottom, upper].map((point) => [point.x, point.y, point.z_top]));
const heights = points.map((point) => planeHeight(plane, [point.x, point.y]));
assert.ok(heights.every((height, index) => Math.abs(height - points[index].z_top) < 0.001));
gameplay.surfaces.push({
  id: "building-088-walk-0",
  node: part.node,
  navigationRegion: "raised-lane-floor",
  polygon: points.map((point) => [point.x, point.y]),
  height: heights,
  holes: [],
  preserveMovementPrecision: true,
});
assert.equal(gameplay.projectionReceivers.filter((entry) => entry.node === part.node).length, 1);
gameplay.projectionReceivers = gameplay.projectionReceivers.filter(
  (entry) => entry.node !== part.node,
);
gameplay.draft.issues.push(
  "Staged sloped lane receiver restoration; verify moved entrances, neighboring floor joins and rendered contact before publication.",
);
validateAssetGameplay(gameplay, descriptor);
const edits = [
  { asset, descriptorSha256: source.descriptor_sha256, modelSha256: source.model_sha256, gameplay },
];
const placement = (id, node) => {
  const matches = scene.placements.filter(
    (entry) => entry.assets.length === 1 && entry.assets[0] === id,
  );
  assert.equal(matches.length, 1);
  assert.equal(matches[0].transform.rot_deg, 0);
  if (node) {
    assert.deepEqual(matches[0].transform, { dx: 0, dy: 0, dz: 0, rot_deg: 0 });
    const transform = matches[0].parts[node].transform;
    assert.equal(transform.rot_deg, 0);
    return transform;
  }
  return matches[0].transform;
};
const lane = placement(asset);
const reconcile = (point, transform) => {
  const height =
    planeHeight(plane, [point[0] + transform.dx - lane.dx, point[1] + transform.dy - lane.dy]) +
    lane.dz -
    transform.dz;
  assert.ok(
    Math.abs(height - point[2]) < 0.001,
    "Anchor reconciliation must stay below 0.001 game units",
  );
  point[2] = height;
};
reconcile(gameplay.jumpZones.find((zone) => zone.id === "jump-zone-60").anchor, lane);
validateAssetGameplay(gameplay, descriptor);
for (const house of [
  "york-west-market-frontage-gabled-house",
  "york-west-market-frontage-dormer-house",
  "york-west-market-frontage-red-roof-house",
  "york-west-market-frontage-north-end-house",
  "york-precinct-stone-causeway",
  "york-light-region-18",
]) {
  const ref = scene.assetSources.find((entry) => entry.id === house);
  const data = await fs.readFile(`library/${ref.descriptor}`);
  assert.equal(hash(data), ref.descriptor_sha256);
  assert.equal(hash(await fs.readFile(`library/${ref.model}`)), ref.model_sha256);
  const definition = JSON.parse(data);
  const local = structuredClone(definition.gameplay);
  const transform = placement(
    house,
    house === "york-light-region-18" ? "scenery-light" : undefined,
  );
  if (house === "york-precinct-stone-causeway") {
    assert.equal(local.doors.length, 7);
    for (const door of local.doors) reconcile(door.outside, transform);
  } else if (house === "york-light-region-18") {
    const light = local.lights.find((light) => light.id === "light-18");
    assert.equal(local.lights.length, 1);
    assert.equal(light.receivers.length, 2);
    reconcile(light.receivers[0], transform);
    // Separate physical footprints retain the same projected coverage at the
    // authored placement while following their receiving floors when rotated.
    const localPlane = [
      plane[0],
      plane[1],
      plane[2] +
        plane[0] * (transform.dx - lane.dx) +
        plane[1] * (transform.dy - lane.dy) +
        lane.dz -
        transform.dz,
    ];
    local.lights = light.receivers.map((receiver, index) => {
      const floor = index === 0 ? localPlane : [0, 0, receiver[2]];
      assert.ok(Math.abs(1 - floor[1]) > 1e-6);
      return {
        ...light,
        id: index === 0 ? light.id : `${light.id}-lower-floor`,
        receivers: [receiver],
        polygon: light.polygon.map(([x, y, z]) => {
          const projectedY = y - z;
          const height = (floor[0] * x + floor[1] * projectedY + floor[2]) / (1 - floor[1]);
          return [x, projectedY + height, height];
        }),
      };
    });
  } else {
    const door = local.interiors
      .find((room) => room.id === "interior-0")
      .doors.find((door) => door.id === "interior-0/door-0");
    reconcile(door.outside, transform);
  }
  validateAssetGameplay(local, definition);
  edits.push({
    asset: house,
    descriptorSha256: ref.descriptor_sha256,
    modelSha256: ref.model_sha256,
    gameplay: local,
  });
}
const output = await fs.mkdtemp("work/needs-review/map-compile/york-raised-lane-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
console.log(output);
