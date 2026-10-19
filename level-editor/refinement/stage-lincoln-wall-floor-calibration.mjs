// Unpublished asset calibration: align two split walkway components with their
// authored mesh height and the adjoining tower entrance with that same floor.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const [stage] = process.argv.slice(2);
assert.ok(stage, "Supply staged plateau and neighboring asset definitions");
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`));
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const changes = [];
const oldHeight = 340.001;
const newHeight = 344.001;
const floors = new Map([
  ["lincoln-east-curtain-wall-south", "building-140--component-east-curtain-south-walk-walk-0"],
  ["lincoln-east-slate-tower", "building-140--component-slate-tower-floor-block-walk-0"],
]);
for (const id of [...floors.keys(), "lincoln-east-gate-north-tower"]) {
  const edit = edits.find((edit) => edit.asset === id);
  assert.ok(edit, `Missing staged asset: ${id}`);
  const root = `library/3d-assets/lincoln/${id}`;
  const bytes = await fs.readFile(`${root}/asset.json`);
  assert.equal(hash(bytes), edit.descriptorSha256);
  const descriptor = JSON.parse(bytes);
  const modelSha256 = hash(await fs.readFile(`${root}/${descriptor.model}`));
  if (edit.modelSha256 !== undefined) assert.equal(edit.modelSha256, modelSha256);
  edit.modelSha256 = modelSha256;
  const gameplay = edit.gameplay;
  const shiftPoint = (point) => {
    assert.ok(Math.abs(point[2] - oldHeight) < 1e-6, "Unexpected contact height");
    point[2] = newHeight;
  };
  if (floors.has(id)) {
    const floor = gameplay.surfaces.find((surface) => surface.id === floors.get(id));
    assert.ok(floor);
    const before = structuredClone(floor);
    const samePlane = (surface) =>
      (Array.isArray(surface.height) ? surface.height : [surface.height]).every(
        (height) => Math.abs(height - oldHeight) < 1e-6,
      );
    const targets = [
      floor,
      ...(gameplay.movementClearances ?? []).filter(
        (clearance) => clearance.node === floor.node && samePlane(clearance),
      ),
    ];
    for (const surface of targets) {
      assert.ok(samePlane(surface));
      assert.equal(surface.navigationHeight, undefined);
      surface.height = Array.isArray(surface.height)
        ? surface.height.map(() => newHeight)
        : newHeight;
      for (const edge of surface.navigationJoins ?? []) for (const point of edge) shiftPoint(point);
      for (const point of surface.projectionMaterials?.planePoints ?? []) shiftPoint(point);
      // The receiving footprint already follows the model cap. Preserve those
      // measured points while correcting its stale fitted navigation plane.
      for (const point of surface.projectionMaterials?.footprint ?? [])
        assert.ok(Math.abs(point[2] - newHeight) < 0.01, "Review receiver footprint height");
      if (surface.projectionMaterials?.priorityHeight !== undefined) {
        assert.equal(surface.projectionMaterials.priorityHeight, oldHeight);
        surface.projectionMaterials.priorityHeight = newHeight;
      }
    }
    changes.push({
      asset: id,
      before,
      after: floor,
      clearances: targets.slice(1).map((s) => s.id),
    });
  } else {
    const door = gameplay.interiors
      .flatMap((room) => room.doors)
      .find((door) => door.id === "interior-0/door-1");
    assert.ok(door && !door.outsideReceiverSegment && !door.insideReceiverSegment);
    const before = structuredClone(door);
    for (const key of ["outside", "middle", "inside", "outsideAnchor", "insideAnchor"])
      if (door[key]) shiftPoint(door[key]);
    changes.push({ asset: id, before, after: door });
  }
  validateAssetGameplay(gameplay, descriptor);
}
const output = await fs.mkdtemp("work/needs-review/map-compile/wall-floor-calibration-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({
    stage,
    oldHeight,
    newHeight,
    published: false,
    changes,
    scope:
      "mesh-height candidate; adjoining floors, moved placements and rendering remain unverified",
  }),
);
console.log(output);
