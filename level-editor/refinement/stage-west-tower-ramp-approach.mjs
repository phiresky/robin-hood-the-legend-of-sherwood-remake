// Unpublished approach experiments; native placement tests must validate each candidate.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";

const [stage, distanceText] = process.argv.slice(2);
const distance = Number(distanceText);
assert.ok(
  stage && Number.isFinite(distance) && distance > 0 && distance <= 32,
  "Supply a gameplay stage and an approach offset in (0, 32]",
);
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`));
const scene = JSON.parse(await fs.readFile("library/scenes/lincoln.rhlos-map.json"));
const ids = ["lincoln-west-round-tower", "lincoln-west-tower-terrace"];
const sources = scene.assetSources.filter((source) => ids.includes(source.id));
const descriptors = await pinnedDescriptors("library", sources);
const selected = ids.map((id) => {
  const edit = edits.find((edit) => edit.asset === id);
  assert.equal(edit.descriptorSha256, sources.find((source) => source.id === id).descriptor_sha256);
  const placement = scene.placements.find((placement) => placement.id === id);
  assert.equal(placement.transform.rot_deg, 0);
  assert.equal(placement.transform.dz, 0);
  return { edit, transform: placement.transform };
});
const [tower, terrace] = selected;
const ramp = terrace.edit.gameplay.surfaces.find((surface) => surface.node === "building-409");
assert.ok(ramp);
const door = tower.edit.gameplay.interiors[0].doors.find((door) => door.id === "interior-0/door-1");
const before = [...door.outside];
door.outside[1] += distance;
assert.equal(door.passageStructure, undefined);
door.passageStructure = { volumes: [door.node], blockers: [] };
const local = [
  door.outside[0] + tower.transform.dx - terrace.transform.dx,
  door.outside[1] + tower.transform.dy - terrace.transform.dy,
];
const floors = terrace.edit.gameplay.surfaces.filter(
  (surface) =>
    pointInGameplayPolygon(local, surface.polygon, true) &&
    !(surface.holes ?? []).some((hole) => pointInGameplayPolygon(local, hole, true)),
);
assert.equal(floors.length, 1);
const floor = floors[0];
const plane = heightPlane(floor.polygon.map(([x, y], i) => [x, y, floor.height[i]]));
door.outside[2] = planeHeight(plane, local);
tower.edit.gameplay.draft.issues.push(
  "Unpublished lower-door approach experiment: native moved placement, receiver picking and passage support remain unverified.",
);
validateAssetGameplay(tower.edit.gameplay, descriptors.get(ids[0]));
const output = await fs.mkdtemp("work/needs-review/map-compile/west-tower-ramp-approach-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({ stage, distance, before, after: door.outside, plane, receiver: floor.id }),
);
console.log(output);
