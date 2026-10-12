// Prepare an unpublished local doorway repair from an editor-authored assembly.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { createHash } from "node:crypto";
import { readStoredMap } from "../pipeline/src/stored-map.ts";
import { gameToScene } from "../shared/src/scene.ts";

const [stepStage] = process.argv.slice(2);
assert.ok(stepStage);
const edits = JSON.parse(await fs.readFile(`${stepStage}/edits.json`));
assert.equal(edits.length, 1);
const step = edits[0];
assert.equal(step.asset, "lincoln-south-wall-walk-step");
const document = await readStoredMap("library/scenes/lincoln.rhlos-map.json", "library");
assert.equal(
  document.assetSources.find((source) => source.id === step.asset)?.descriptor_sha256,
  step.descriptorSha256,
);
const asset = "lincoln-south-wall-cone-turret";
const turret = document.groups.find((group) => group.id === asset);
const stair = document.groups.find((group) => group.id === step.asset);
assert.equal(turret.transform.rot_deg, 0);
assert.equal(stair.transform.rot_deg, 0);
const source = document.assetSources.find((source) => source.id === asset);
const descriptorBytes = await fs.readFile(path.join("library", source.descriptor));
const modelBytes = await fs.readFile(path.join("library", source.model));
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
assert.equal(hash(descriptorBytes), source.descriptor_sha256);
assert.equal(hash(modelBytes), source.model_sha256);
const low = step.gameplay.lifts[0].doors[0];
const local = (point) =>
  point.map(
    (value, axis) =>
      value +
      stair.transform[["dx", "dy", "dz"][axis]] -
      turret.transform[["dx", "dy", "dz"][axis]],
  );
const outside = local(low.outside),
  middle = local(low.middle);
assert.ok(Math.abs(outside[2] - middle[2]) < 1e-6);
const length = Math.hypot(middle[0] - outside[0], middle[1] - outside[1]);
const direction = middle.slice(0, 2).map((value, axis) => (value - outside[axis]) / length);
const normal = [-direction[1], direction[0]];
const width = 10;
const corner = (along, across) =>
  outside.slice(0, 2).map((value, axis) => value + direction[axis] * along + normal[axis] * across);
const polygon = [
  corner(-24, -width),
  corner(length + 1, -width),
  corner(length + 1, width),
  corner(-24, width),
];
const z = outside[2];
assert.ok(Math.abs(z - 320.001) < 1e-6, "Review the doorway heights if the assembly has changed");
const prism = (bottom, top) =>
  [bottom, top].flatMap((height) =>
    polygon.map(([x, y]) => gameToScene(document.camera, x, y, height)),
  );
const output = await fs.mkdtemp("work/needs-review/map-compile/turret-opening-");
await fs.writeFile(
  `${output}/opening.json`,
  JSON.stringify({
    scope: "unpublished-mesh-authoring-candidate-not-gameplay-or-visual-parity",
    stepStage,
    asset,
    source,
    inputModel: path.resolve("library", source.model),
    outputModel: path.resolve(output, "model.glb"),
    camera: document.camera,
    cutter: prism(z - 0.1, 356.01),
    floor: prism(z - 4, z),
    polygon,
    floorHeight: z,
    doorwayTop: 356.01,
    nodes: ["building-104--component-door-turret-wall", "building-127--component-door-turret-wall"],
    floorNode: "building-127--component-door-turret-wall",
  }),
);
console.log(output);
