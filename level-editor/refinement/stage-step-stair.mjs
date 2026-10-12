// Unpublished explicit stair candidate from the step's own receiving plane.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { createHash } from "node:crypto";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
const asset = "lincoln-south-wall-walk-step";
const root = `library/3d-assets/lincoln/${asset}`;
const bytes = await fs.readFile(`${root}/asset.json`);
const descriptor = JSON.parse(bytes);
const gameplay = structuredClone(descriptor.gameplay);
assert.equal(gameplay.lifts.length, 0);
assert.equal(gameplay.doors.length, 1);
const surface = gameplay.surfaces[0];
const originalDoor = gameplay.doors[0];
const plane = heightPlane(surface.projectionMaterials.planePoints);
const direction = [plane[0], plane[1]].map((v) => v / Math.hypot(plane[0], plane[1]));
const seat = (point, height) => {
  const t = (height - planeHeight(plane, point)) / (plane[0] ** 2 + plane[1] ** 2);
  return [point[0] + t * plane[0], point[1] + t * plane[1], height];
};
const footprint = surface.projectionMaterials.footprint;
const lowVertices = footprint.filter((p) => Math.abs(p[2] - 320.001) < 0.01);
const highVertices = footprint.filter((p) => Math.abs(p[2] - 350.001) < 0.01);
assert.equal(lowVertices.length, 2);
assert.equal(highVertices.length, 2);
const midpoint = (points) => points[0].map((v, i) => (v + points[1][i]) / 2);
const low = seat(midpoint(lowVertices), 320.001);
const high = seat(midpoint(highVertices), 350.001);
const inset = (point, distance) => {
  const xy = point.slice(0, 2).map((v, i) => v + distance * direction[i]);
  return [...xy, planeHeight(plane, xy)];
};
surface.polygon = footprint.map((point) => seat(point, point[2]).slice(0, 2));
surface.height = surface.polygon.map((point) => planeHeight(plane, point));
surface.projectionMaterials.footprint = surface.polygon.map((point, i) => [
  ...point,
  surface.height[i],
]);
surface.preserveMovementPrecision = true;
const id = `${surface.node}-lift`;
gameplay.doors = [];
gameplay.lifts = [
  {
    id,
    node: surface.node,
    surface: surface.id,
    type: 1,
    direction,
    doors: [
      {
        ...originalDoor,
        id: `${id}/low`,
        type: 5,
        middle: low,
        inside: inset(low, 8),
        outside: originalDoor.outside,
      },
      {
        ...originalDoor,
        id: `${id}/high`,
        type: 4,
        middle: high,
        inside: inset(high, -8),
        outside: [...inset(high, 10).slice(0, 2), high[2]],
      },
    ].map(({ insideAnchor: _insideAnchor, allowContinuous: _allowContinuous, ...door }) => door),
  },
];
gameplay.draft.issues.push(
  "Unpublished explicit step stair: validate both landings, rotated clicks and independently placed turret clearance.",
);
validateAssetGameplay(gameplay, descriptor);
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const output = await fs.mkdtemp("work/needs-review/map-compile/step-stair-");
await fs.writeFile(
  `${output}/edits.json`,
  JSON.stringify([
    {
      asset,
      descriptorSha256: hash(bytes),
      modelSha256: hash(await fs.readFile(`${root}/model.glb`)),
      gameplay,
    },
  ]),
);
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({ low, high, direction, published: false }),
);
const scene = JSON.parse(await fs.readFile("library/scenes/lincoln.rhlos-map.json"));
await fs.writeFile(
  `${output}/mesh-plane.json`,
  JSON.stringify({
    scope: "unpublished-step-mesh-plane-repair",
    inputModel: path.resolve(`${root}/model.glb`),
    modelSha256: hash(await fs.readFile(`${root}/model.glb`)),
    modelScene: descriptor.model_scene ?? "default",
    outputModel: path.resolve(`${output}/model.glb`),
    node: surface.node,
    camera: scene.camera,
    footprint: surface.projectionMaterials.footprint,
    maximumDisplacement: 1,
  }),
);
console.log(output);
