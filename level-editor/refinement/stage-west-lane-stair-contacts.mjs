import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { compileMap } from "../app/src/map-compile.ts";

// Each receiving asset owns its contact; placement still determines connectivity.
const [stage] = process.argv.slice(2);
assert.ok(stage);
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
assert.equal(edits.length, 1);
assert.equal(edits[0].asset, "york-west-lane-access-steps");
const document = await readStoredMap("library/scenes/york.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
const placement = document.groups.find((group) => group.id === edits[0].asset).transform;
assert.equal(placement.rot_deg, 0);
const stair = edits[0].gameplay;
const floor = stair.surfaces.find((surface) => surface.id === "building-106-walk-0");
const plane = heightPlane(
  floor.polygon.map(([x, y], i) => [
    x + placement.dx,
    y + placement.dy,
    floor.height[i] + placement.dz,
  ]),
);
const changes = [];
for (const contact of [
  {
    asset: "york-southeast-riverside-raised-terrace",
    surface: "building-093-physical-walkway",
    volume: "building-093-reviewed-receiver",
    receiverHeight: 0,
    corners: [
      [-226.97699999999986, -20.449545571331782],
      [-157.01119999999992, -20.256245571331874],
    ],
  },
  {
    asset: "york-terrain",
    surface: "ground-section-3-0",
    receiverHeight: 50.001003,
    corners: [
      [2304, 1236],
      [2370, 1236],
    ],
  },
  {
    asset: "york-east-bridge-raised-terrace",
    surface: "building-092-walk-0",
    receiverHeight: 0,
    corners: [
      [145, 103.68914704261964],
      [103, 103.68914704261964],
    ],
  },
]) {
  const descriptor = assets.get(contact.asset);
  const gameplay = structuredClone(descriptor.gameplay);
  const surface = gameplay.surfaces.find((surface) => surface.id === contact.surface);
  assert.ok(surface);
  const origin = document.groups.find((group) => group.id === contact.asset)?.transform ?? {
    dx: 0,
    dy: 0,
    dz: 0,
    rot_deg: 0,
  };
  assert.equal(origin.rot_deg, 0);
  for (const corner of contact.corners) {
    const i = surface.polygon.findIndex(
      (point) => Math.hypot(...point.map((v, axis) => v - corner[axis])) < 1e-6,
    );
    assert.ok(i >= 0, "Reviewed receiving corner changed");
    const before = surface.polygon[i];
    const world = [before[0] + origin.dx, before[1] + origin.dy + contact.receiverHeight];
    const height =
      (Array.isArray(surface.height) ? surface.height[i] : surface.height) +
      origin.dz +
      contact.receiverHeight;
    const t = (height - planeHeight(plane, world)) / (plane[0] ** 2 + plane[1] ** 2);
    const distance = Math.abs(t) * Math.hypot(plane[0], plane[1]);
    assert.ok(distance < 2, "Receiving contact exceeds reviewed authoring bound");
    surface.polygon[i] = before.map((value, axis) => value + t * plane[axis]);
    if (contact.volume) {
      const volume = gameplay.volumes.find((volume) => volume.id === contact.volume);
      assert.ok(volume, "Physical receiving volume changed");
      const point = volume.shape.points.find(
        (point) => Math.hypot(point.x - before[0], point.y - before[1]) < 1e-6,
      );
      assert.ok(point, "Physical receiving contour changed");
      [point.x, point.y] = surface.polygon[i];
    }
    changes.push({
      asset: contact.asset,
      surface: surface.id,
      before,
      after: surface.polygon[i],
      distance,
    });
  }
  surface.preserveMovementPrecision = true;
  gameplay.draft.issues.push(
    "West-lane stair contact is asset-local and requires rendered integration review.",
  );
  validateAssetGameplay(gameplay, descriptor);
  edits.push({
    asset: descriptor.id,
    descriptorSha256: index.find((entry) => entry.id === descriptor.id).descriptor_sha256,
    gameplay,
  });
}
const output = await fs.mkdtemp("work/map-compile/west-lane-stair-contacts-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(`${output}/review.json`, JSON.stringify({ stage, changes }));
console.log(JSON.stringify({ output, changes }));
for (const edit of edits) assets.get(edit.asset).gameplay = edit.gameplay;
const compiled = compileMap(document, [0, 0, ...document.size], assets, { bestEffort: true });
await fs.writeFile(`${output}/york.level.json`, JSON.stringify(compiled.descriptor));
await fs.writeFile(
  `${output}/diagnostics.json`,
  JSON.stringify({
    scope: "static-geometry-only-not-gameplay-parity",
    complete: true,
    results: [{ map: "york", file: "york.level.json", warnings: compiled.warnings }],
  }),
);
console.log(JSON.stringify({ output, complete: true }));
