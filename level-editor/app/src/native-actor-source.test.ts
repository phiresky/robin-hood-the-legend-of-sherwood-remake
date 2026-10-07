import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { bindInitialActorSource, queryActorMasks } from "./native-actor-source.ts";
import type { Mask } from "../../shared/src/level.ts";
const root = resolve(import.meta.dirname, "../../..");
const read = (path: string) => JSON.parse(readFileSync(resolve(root, path), "utf8"));
const contract = read(
  "level-editor/library/mission-states/croisement02/contracts/signposts.json",
).native;
const source = {
  name: "S03_FoB_MP",
  data: read("level-editor/library/mission-states/croisement02/source/S03_FoB_MP.rhm.json"),
  level: read("level-editor/library/mission-states/croisement02/source/Croisement02.rhp.json"),
  camera: { kind: "oblique-orthographic" as const, elevation_deg: 35 },
};
test("actual S03 stream includes invisible patches and binds layer-local mask refs", async () => {
  const got = await bindInitialActorSource(source, contract, 1);
  assert.equal(got.authority.creationRanks.get("civilians:0"), 25);
  assert.equal(got.authority.creationRanks.get("pcs_to_rescue:0"), 39);
  assert.equal(got.authority.creationRanks.get("soldiers:0"), 40);
  assert.equal(got.authority.creationRanks.get("mission-target:4"), 104);
  assert.equal(got.authority.creationRanks.get("mission-target:8"), 108);
  assert.deepEqual(
    got.membership.flatMap((active, i) => (active ? [] : [i])),
    [138, 139, 140, 141],
  );
  assert.deepEqual(got.gridSize, [28, 22]);
  assert.ok(got.authority.backgroundEffects.has("map-animation:12"));
  const changed = structuredClone(source);
  changed.data.soldiers[0].direction = 0;
  if (changed.data.soldiers[0].direction === source.data.soldiers[0].direction)
    changed.data.soldiers[0].direction = 1;
  await assert.rejects(() => bindInitialActorSource(changed, contract, 1), /source revision/);
});
test("cell-first mask order can differ from global index order", () => {
  const mask = (x: number): Mask => ({
    layer: 0,
    mask_type: 1,
    character_polyline: [
      [0, 100],
      [200, 100],
    ],
    projectile_polyline: null,
    box_top_left: [x, 0],
    box_size: [10, 10],
    mask_data: [],
    obstacle_indices: [],
  });
  const masks = [mask(70), mask(0)];
  assert.deepEqual(
    queryActorMasks(
      masks,
      [true, true],
      { layer: 0, mapPosition: [5, 0] },
      [0, 0, 80, 10],
      [3, 5],
    ).map((m) => m.id),
    ["mask:1", "mask:0"],
  );
  assert.deepEqual(
    queryActorMasks(
      masks,
      [true, false],
      { layer: 0, mapPosition: [5, 0] },
      [0, 0, 80, 10],
      [3, 5],
    ).map((m) => m.id),
    ["mask:0"],
  );
});
